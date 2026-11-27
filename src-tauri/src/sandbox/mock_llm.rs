//! 诊断用本地 Anthropic 兼容假上游。
//!
//! 只由 `--sandbox-exec` 显式开启，用于验证 Claude CLI 的 prompt 能经过
//! 容器 relay 发到宿主侧的 `/proxy/<token>/v1/messages`，并正确消费回复。
//! 它不连接公网，也不模拟真实模型能力。

use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::any,
    Json, Router,
};
use rand::{distributions::Alphanumeric, Rng};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

const ANSWER_PREFIX: &str = "LUMEN_SANDBOX_OK:";

struct MockContext {
    token: String,
    nonce: String,
    record: Mutex<MockRecord>,
}

#[derive(Default)]
struct MockRecord {
    requests: usize,
    accepted: usize,
    rejected: usize,
    prompt_nonce_seen: bool,
    auth_header_present: bool,
    model: Option<String>,
    stream: bool,
    paths: Vec<String>,
}

/// 报告只记录诊断所需的最小信息，不回显鉴权头或请求正文。
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MockLlmReport {
    pub requests: usize,
    pub accepted: usize,
    pub rejected: usize,
    pub prompt_nonce_seen: bool,
    pub auth_header_present: bool,
    pub model: Option<String>,
    pub stream: bool,
    pub paths: Vec<String>,
    pub answer: String,
}

pub struct MockLlm {
    pub port: u16,
    pub token: String,
    context: Arc<MockContext>,
}

impl MockLlm {
    pub fn report(&self) -> MockLlmReport {
        let answer = format!("{ANSWER_PREFIX}{}", self.context.nonce);
        let record = self
            .context
            .record
            .lock()
            .map(|record| MockRecord {
                requests: record.requests,
                accepted: record.accepted,
                rejected: record.rejected,
                prompt_nonce_seen: record.prompt_nonce_seen,
                auth_header_present: record.auth_header_present,
                model: record.model.clone(),
                stream: record.stream,
                paths: record.paths.clone(),
            })
            .unwrap_or_default();
        MockLlmReport {
            requests: record.requests,
            accepted: record.accepted,
            rejected: record.rejected,
            prompt_nonce_seen: record.prompt_nonce_seen,
            auth_header_present: record.auth_header_present,
            model: record.model,
            stream: record.stream,
            paths: record.paths,
            answer,
        }
    }
}

/// 启动假上游；监听端口由系统分配，只绑定回环地址。
pub fn start(nonce: String) -> Result<MockLlm, String> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| format!("启动本地假模型上游失败：{e}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("设置本地假模型上游非阻塞失败：{e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("读取本地假模型上游端口失败：{e}"))?
        .port();
    let token: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(48)
        .map(char::from)
        .collect();
    let context = Arc::new(MockContext {
        token: token.clone(),
        nonce,
        record: Mutex::new(MockRecord::default()),
    });
    let router = Router::new()
        .fallback(any(handle))
        .with_state(context.clone());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("初始化本地假模型上游运行时失败：{e}"))?;
    std::thread::Builder::new()
        .name("lumen-mock-llm".to_string())
        .spawn(move || {
            runtime.block_on(async move {
                let listener = match tokio::net::TcpListener::from_std(listener) {
                    Ok(listener) => listener,
                    Err(err) => {
                        eprintln!("mock llm listener error: {err}");
                        return;
                    }
                };
                let _ = axum::serve(listener, router).await;
            });
        })
        .map_err(|e| format!("启动本地假模型上游线程失败：{e}"))?;
    Ok(MockLlm {
        port,
        token,
        context,
    })
}

async fn handle(
    State(ctx): State<Arc<MockContext>>,
    _method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = uri.path().to_string();
    let auth_header_present =
        headers.contains_key("authorization") || headers.contains_key("x-api-key");
    let prompt_nonce_seen = String::from_utf8_lossy(&body).contains(&ctx.nonce);
    let parsed = serde_json::from_slice::<Value>(&body).ok();
    let model = parsed
        .as_ref()
        .and_then(|value| value.get("model"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let stream = parsed
        .as_ref()
        .and_then(|value| value.get("stream"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let prefix = format!("/proxy/{}/", ctx.token);
    let Some(api_path) = path.strip_prefix(&prefix) else {
        if let Ok(mut record) = ctx.record.lock() {
            record.requests += 1;
            record.rejected += 1;
            record.auth_header_present |= auth_header_present;
            if record.paths.len() < 8 {
                record.paths.push(path);
            }
        }
        return anthropic_error(
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            "诊断令牌或代理路径不匹配",
        );
    };

    {
        let Ok(mut record) = ctx.record.lock() else {
            return anthropic_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "api_error",
                "假上游状态锁损坏",
            );
        };
        record.requests += 1;
        record.auth_header_present |= auth_header_present;
        record.prompt_nonce_seen |= prompt_nonce_seen;
        record.stream |= stream;
        if model.is_some() {
            record.model = model.clone();
        }
        if record.paths.len() < 8 {
            record.paths.push(path.clone());
        }
    }

    if api_path == "v1/messages/count_tokens" {
        if let Ok(mut record) = ctx.record.lock() {
            record.accepted += 1;
        }
        return Json(json!({ "input_tokens": 16 })).into_response();
    }

    if let Ok(mut record) = ctx.record.lock() {
        record.accepted += 1;
    }
    let answer = format!("{ANSWER_PREFIX}{}", ctx.nonce);
    let response_model = model.unwrap_or_else(|| "claude-lumen-diagnostic".to_string());
    if stream {
        return sse_response(&response_model, &answer);
    }
    Json(json!({
        "id": "msg_lumen_diagnostic",
        "type": "message",
        "role": "assistant",
        "model": response_model,
        "content": [{ "type": "text", "text": answer }],
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": { "input_tokens": 16, "output_tokens": 8 }
    }))
    .into_response()
}

fn sse_response(model: &str, answer: &str) -> Response {
    let events = [
        (
            "message_start",
            json!({
                "type": "message_start",
                "message": {
                    "id": "msg_lumen_diagnostic",
                    "type": "message",
                    "role": "assistant",
                    "model": model,
                    "content": [],
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": { "input_tokens": 16, "output_tokens": 1 }
                }
            }),
        ),
        (
            "content_block_start",
            json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": { "type": "text", "text": "" }
            }),
        ),
        (
            "content_block_delta",
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "text_delta", "text": answer }
            }),
        ),
        (
            "content_block_stop",
            json!({ "type": "content_block_stop", "index": 0 }),
        ),
        (
            "message_delta",
            json!({
                "type": "message_delta",
                "delta": { "stop_reason": "end_turn", "stop_sequence": null },
                "usage": { "output_tokens": 8 }
            }),
        ),
        ("message_stop", json!({ "type": "message_stop" })),
    ];
    let mut payload = String::new();
    for (event, data) in events {
        let Ok(data) = serde_json::to_string(&data) else {
            return anthropic_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "api_error",
                "序列化假上游 SSE 失败",
            );
        };
        payload.push_str(&format!("event: {event}\ndata: {data}\n\n"));
    }
    let mut response = Response::new(Body::from(payload));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

fn anthropic_error(status: StatusCode, kind: &str, message: &str) -> Response {
    (
        status,
        Json(json!({
            "type": "error",
            "error": { "type": kind, "message": message }
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn serves_nonstream_message_and_records_only_metadata() {
        let nonce = "lumen-diagnostic-nonce";
        let mock = start(nonce.to_string()).unwrap();
        let url = format!(
            "http://127.0.0.1:{}/proxy/{}/v1/messages",
            mock.port, mock.token
        );
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(5))
            .build();
        let response = agent
            .post(&url)
            .set("authorization", "Bearer diagnostic-token")
            .send_json(json!({
                "model": "claude-lumen-diagnostic",
                "max_tokens": 64,
                "stream": false,
                "messages": [{ "role": "user", "content": nonce }]
            }))
            .unwrap();
        assert_eq!(response.status(), 200);
        let payload: Value = response.into_json().unwrap();
        assert_eq!(
            payload["content"][0]["text"],
            format!("{ANSWER_PREFIX}{nonce}")
        );

        let report = mock.report();
        assert_eq!(report.requests, 1);
        assert_eq!(report.accepted, 1);
        assert!(report.prompt_nonce_seen);
        assert!(report.auth_header_present);
        assert_eq!(report.model.as_deref(), Some("claude-lumen-diagnostic"));
        assert!(!report.stream);
    }

    #[test]
    fn rejects_requests_outside_the_diagnostic_proxy_prefix() {
        let mock = start("nonce".to_string()).unwrap();
        let url = format!("http://127.0.0.1:{}/v1/messages", mock.port);
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(5))
            .build();
        let status = match agent.post(&url).send_json(json!({ "messages": [] })) {
            Ok(response) => response.status(),
            Err(ureq::Error::Status(status, _)) => status,
            Err(err) => panic!("假上游请求意外失败：{err}"),
        };
        assert_eq!(status, 401);
        let report = mock.report();
        assert_eq!(report.requests, 1);
        assert_eq!(report.rejected, 1);
    }
}
