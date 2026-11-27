use crate::budget::{self, BudgetScope};
use crate::db::{log_event, Db};
use crate::error::{AppError, Result};
use crate::runtime::RunRegistry;
use crate::usage::{self, UsageInput};
use axum::{
    body::{Body, Bytes},
    extract::{Path, State},
    http::{header::CONTENT_TYPE, HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::any,
    Json, Router,
};
use futures_util::StreamExt;
use rand::{distributions::Alphanumeric, Rng};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

/// 一次运行的代理上下文：真实 API Key 只在这里解密并持有，
/// 子进程只能拿到 127.0.0.1 上的一次性令牌地址。
#[derive(Debug, Clone)]
pub struct ProxyRun {
    pub run_id: String,
    pub agent_id: String,
    pub project_id: String,
    pub team_id: String,
    pub task_id: String,
    pub seat_id: Option<String>,
    pub protocol: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub notify: bool,
}

#[derive(Default)]
pub struct ProxyState {
    pub port: u16,
    runs: Mutex<HashMap<String, Arc<ProxyRun>>>,
    by_run: Mutex<HashMap<String, String>>,
    recorded: Mutex<HashMap<String, u64>>,
    blocked: Mutex<HashSet<String>>,
}

impl ProxyState {
    pub fn register(&self, run: ProxyRun) -> String {
        let token: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(48)
            .map(char::from)
            .collect();
        if let Ok(mut runs) = self.runs.lock() {
            runs.insert(token.clone(), Arc::new(run.clone()));
        }
        if let Ok(mut index) = self.by_run.lock() {
            index.insert(run.run_id.clone(), token.clone());
        }
        token
    }

    pub fn unregister(&self, run_id: &str) {
        let token = self
            .by_run
            .lock()
            .ok()
            .and_then(|mut map| map.remove(run_id));
        if let Some(token) = token {
            if let Ok(mut runs) = self.runs.lock() {
                runs.remove(&token);
            }
        }
        if let Ok(mut recorded) = self.recorded.lock() {
            recorded.remove(run_id);
        }
        if let Ok(mut blocked) = self.blocked.lock() {
            blocked.remove(run_id);
        }
    }

    fn lookup(&self, token: &str) -> Option<Arc<ProxyRun>> {
        self.runs
            .lock()
            .ok()
            .and_then(|runs| runs.get(token).cloned())
    }

    fn note_recorded(&self, run_id: &str) -> u64 {
        let mut recorded = match self.recorded.lock() {
            Ok(r) => r,
            Err(_) => return 1,
        };
        let slot = recorded.entry(run_id.to_string()).or_insert(0);
        *slot += 1;
        *slot
    }

    /// runtime 用它判断该运行的用量是否已由代理记录，避免重复计费。
    pub fn usage_recorded(&self, run_id: &str) -> bool {
        self.recorded
            .lock()
            .map(|r| r.get(run_id).copied().unwrap_or(0) > 0)
            .unwrap_or(false)
    }

    fn mark_blocked(&self, run_id: &str) {
        if let Ok(mut blocked) = self.blocked.lock() {
            blocked.insert(run_id.to_string());
        }
    }

    /// 本次运行是否被代理以 402 硬停过，用于最终状态判定为 paused_budget。
    pub fn was_blocked(&self, run_id: &str) -> bool {
        self.blocked
            .lock()
            .map(|b| b.contains(run_id))
            .unwrap_or(false)
    }
}

struct ProxyCtx {
    proxy: Arc<ProxyState>,
    db: Db,
    app: AppHandle,
    registry: Arc<RunRegistry>,
    client: reqwest::Client,
}

pub fn start_server(app: AppHandle, db: Db, registry: Arc<RunRegistry>) -> Result<Arc<ProxyState>> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let state = Arc::new(ProxyState {
        port,
        ..Default::default()
    });
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| AppError::Msg(format!("初始化 LLM 代理失败：{e}")))?;
    let ctx = Arc::new(ProxyCtx {
        proxy: state.clone(),
        db,
        app,
        registry,
        client,
    });
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(l) => l,
            Err(err) => {
                tracing::error!("llm proxy listener error: {err}");
                return;
            }
        };
        let router = Router::new()
            .route("/proxy/{token}/{*path}", any(handle))
            .with_state(ctx);
        if let Err(err) = axum::serve(listener, router).await {
            tracing::error!("llm proxy stopped: {err}");
        }
    });
    Ok(state)
}

async fn handle(
    State(ctx): State<Arc<ProxyCtx>>,
    Path((token, path)): Path<(String, String)>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(run) = ctx.proxy.lookup(&token) else {
        return json_status(StatusCode::UNAUTHORIZED, "代理令牌无效或已过期");
    };
    // 只允许规范的相对路径，阻断用 ../ 或 // 跳到上游其他端点的尝试。
    if path.contains("..") || path.starts_with('/') || path.contains("://") {
        return json_status(StatusCode::BAD_REQUEST, "非法代理路径");
    }
    let is_llm = method == Method::POST && is_llm_path(&path);
    let mut payload = body.clone();
    let mut reservations: Vec<String> = Vec::new();

    if is_llm {
        if let Ok(mut value) = serde_json::from_slice::<Value>(&body) {
            if value.is_object() {
                match preflight(&ctx, &run, &mut value, body.len()) {
                    Ok(ids) => reservations = ids,
                    Err(response) => return response,
                }
                payload = Bytes::from(value.to_string());
            }
        }
    }

    let url = join_upstream(&run.base_url, &path, uri.query());
    let mut request = ctx.client.request(method.clone(), &url);
    for (name, value) in headers.iter() {
        let lower = name.as_str().to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "host"
                | "content-length"
                | "authorization"
                | "x-api-key"
                | "accept-encoding"
                | "connection"
                | "transfer-encoding"
                | "proxy-connection"
        ) {
            continue;
        }
        if let Ok(text) = value.to_str() {
            request = request.header(name, text);
        }
    }
    if !run.api_key.is_empty() {
        if run.protocol == "openai" {
            request = request.header("authorization", format!("Bearer {}", run.api_key));
        } else {
            // 同时兼容官方 x-api-key 与第三方 Anthropic 兼容网关的 Bearer 鉴权。
            request = request
                .header("x-api-key", &run.api_key)
                .header("authorization", format!("Bearer {}", run.api_key));
        }
    }

    let upstream = match request.body(payload.clone()).send().await {
        Ok(resp) => resp,
        Err(err) => {
            settle_now(&ctx, &reservations);
            return json_status(
                StatusCode::BAD_GATEWAY,
                &format!("上游模型服务不可达：{err}"),
            );
        }
    };

    let status =
        StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let content_type = upstream
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    let is_sse = content_type.starts_with("text/event-stream");

    if !is_sse {
        let bytes = match upstream.bytes().await {
            Ok(b) => b,
            Err(err) => {
                settle_now(&ctx, &reservations);
                return json_status(StatusCode::BAD_GATEWAY, &format!("读取上游响应失败：{err}"));
            }
        };
        if is_llm && status.is_success() {
            let mut scan = UsageScan::default();
            scan.feed(&bytes);
            record_usage(&ctx, &run, scan, body.len(), bytes.len(), &reservations);
        } else {
            settle_now(&ctx, &reservations);
        }
        return Response::builder()
            .status(status)
            .header(CONTENT_TYPE, content_type.clone())
            .body(Body::from(bytes))
            .unwrap_or_else(|_| json_status(StatusCode::BAD_GATEWAY, "响应构建失败"));
    }

    // 流式响应：逐块转发并增量解析 usage，保证 UI 不等待整段回复。
    let ctx2 = ctx.clone();
    let run2 = run.clone();
    let ids = reservations.clone();
    let req_len = body.len();
    let ok = status.is_success();
    let stream = async_stream::stream! {
        let mut upstream = upstream.bytes_stream();
        let mut scan = UsageScan::default();
        let mut resp_len = 0usize;
        while let Some(chunk) = upstream.next().await {
            match chunk {
                Ok(bytes) => {
                    resp_len += bytes.len();
                    scan.feed(&bytes);
                    yield Ok::<Bytes, std::io::Error>(bytes);
                }
                Err(err) => {
                    yield Err(std::io::Error::other(err.to_string()));
                    break;
                }
            }
        }
        if ok {
            record_usage(&ctx2, &run2, scan, req_len, resp_len, &ids);
        } else {
            settle_now(&ctx2, &ids);
        }
    };

    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .header("cache-control", "no-cache")
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| json_status(StatusCode::BAD_GATEWAY, "流式响应构建失败"))
}

fn is_llm_path(path: &str) -> bool {
    let path = path.trim_end_matches('/');
    path.ends_with("/messages")
        || path.ends_with("/chat/completions")
        || path.ends_with("/completions")
}

fn join_upstream(base: &str, path: &str, query: Option<&str>) -> String {
    let base = base.trim_end_matches('/').to_string();
    let mut path = path.trim_start_matches('/').to_string();
    if base.ends_with("/v1") && path.starts_with("v1/") {
        path = path[3..].to_string();
    }
    let mut url = format!("{base}/{path}");
    if let Some(query) = query.filter(|q| !q.is_empty()) {
        url.push('?');
        url.push_str(query);
    }
    url
}

/// 发送请求前的预算准入：先按剩余额度下调 max_tokens，仍不足则 402 + 终止运行。
#[allow(clippy::result_large_err)]
fn preflight(
    ctx: &ProxyCtx,
    run: &ProxyRun,
    value: &mut Value,
    body_len: usize,
) -> std::result::Result<Vec<String>, Response> {
    let conn = ctx
        .db
        .open()
        .map_err(|e| json_status(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let scope = BudgetScope {
        project_id: &run.project_id,
        team_id: &run.team_id,
        agent_id: &run.agent_id,
        task_id: &run.task_id,
        run_id: &run.run_id,
    };
    let est_input = ((body_len / 4).max(1)) as i64;
    let requested = value
        .get("max_tokens")
        .or_else(|| value.get("max_completion_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(4096)
        .max(1);
    let model = value
        .get("model")
        .and_then(|v| v.as_str())
        .filter(|m| !m.trim().is_empty())
        .unwrap_or(&run.model)
        .to_string();
    let provider = usage::provider_for(&model);
    let (_, out_price, _, _) = usage::price_for(&conn, &provider, &model)
        .unwrap_or((3_000_000, 15_000_000, 300_000, 3_750_000));
    let headroom = budget::headroom(&conn, &scope);
    let allowed = clamp_output(headroom, est_input, requested, out_price);

    if allowed < 64 {
        ctx.proxy.mark_blocked(&run.run_id);
        let _ = log_event(
            &conn,
            Some(&run.project_id),
            Some(&run.task_id),
            Some(&run.run_id),
            "budget.blocked",
            Some("llm-proxy"),
            json!({ "model": model, "requestedMaxTokens": requested, "headroom": headroom }),
        );
        let _ = ctx.app.emit(
            "run.status",
            json!({
                "runId": run.run_id,
                "status": "paused_budget",
                "taskId": run.task_id,
                "error": "预算已用尽，请求被拦截"
            }),
        );
        budget::enforce(&ctx.app, &conn, &scope, &ctx.registry, run.notify);
        return Err(budget_error("Lumen 预算已用尽，本次模型请求被硬停"));
    }

    if allowed < requested {
        value["max_tokens"] = json!(allowed);
        if value.get("max_completion_tokens").is_some() {
            value["max_completion_tokens"] = json!(allowed);
        }
    }

    let est_tokens = est_input + allowed;
    let est_micros =
        usage::compute_cost_micros(&conn, &provider, &model, est_input, allowed, 0, 0).unwrap_or(0);
    // 没有硬停预算时直接放行，避免每个模型请求都拿一次写锁。
    if headroom.usd_micros.is_none() && headroom.tokens.is_none() {
        return Ok(Vec::new());
    }
    match budget::admit(&conn, &scope, est_micros, est_tokens) {
        Ok(ids) => Ok(ids),
        Err(err) => {
            ctx.proxy.mark_blocked(&run.run_id);
            let _ = log_event(
                &conn,
                Some(&run.project_id),
                Some(&run.task_id),
                Some(&run.run_id),
                "budget.blocked",
                Some("llm-proxy"),
                json!({ "model": model, "reason": err.to_string() }),
            );
            let _ = ctx.app.emit(
                "run.status",
                json!({
                    "runId": run.run_id,
                    "status": "paused_budget",
                    "taskId": run.task_id,
                    "error": err.to_string()
                }),
            );
            budget::enforce(&ctx.app, &conn, &scope, &ctx.registry, run.notify);
            Err(budget_error(&format!(
                "Lumen 预算不足，本次模型请求被拦停：{err}"
            )))
        }
    }
}

fn clamp_output(headroom: budget::Headroom, est_input: i64, requested: i64, out_price: i64) -> i64 {
    let mut allowed = requested.max(1);
    if let Some(tokens) = headroom.tokens {
        allowed = allowed.min((tokens - est_input).max(0));
    }
    if let Some(micros) = headroom.usd_micros {
        if out_price > 0 {
            let afford = (micros as i128 * 1_000_000 / out_price as i128) as i64;
            allowed = allowed.min((afford - est_input).max(0));
        } else if micros <= 0 {
            allowed = 0;
        }
    }
    allowed
}

fn settle_now(ctx: &ProxyCtx, ids: &[String]) {
    if ids.is_empty() {
        return;
    }
    if let Ok(conn) = ctx.db.open() {
        budget::settle(&conn, ids);
    }
}

fn record_usage(
    ctx: &ProxyCtx,
    run: &ProxyRun,
    mut scan: UsageScan,
    req_len: usize,
    resp_len: usize,
    ids: &[String],
) {
    scan.finish();
    let Ok(conn) = ctx.db.open() else {
        return;
    };
    // 先结算预留再落实际用量，避免“预留 + 实耗”短暂叠加导致预算误判为超限。
    budget::settle(&conn, ids);
    let (input, output, cache_read, cache_write) = if scan.total() > 0 {
        (scan.input, scan.output, scan.cache_read, scan.cache_write)
    } else {
        // 上游未返回 usage 时退回字符估算，并标记来源，避免预算与看板长期失真。
        (((req_len / 4).max(1)) as i64, (resp_len / 4) as i64, 0, 0)
    };
    if input + output + cache_read + cache_write == 0 {
        return;
    }
    let model = if scan.model.is_empty() {
        run.model.clone()
    } else {
        scan.model.clone()
    };
    let seq = ctx.proxy.note_recorded(&run.run_id);
    let event = UsageInput {
        run_id: Some(run.run_id.clone()),
        task_id: Some(run.task_id.clone()),
        project_id: Some(run.project_id.clone()),
        team_id: Some(run.team_id.clone()),
        seat_id: run.seat_id.clone(),
        agent_id: Some(run.agent_id.clone()),
        provider: usage::provider_for(&model),
        model: model.clone(),
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        source: "proxy".into(),
        dedupe_key: format!("proxy:{}:{}", run.run_id, seq),
    };
    if let Ok(Some(cost)) = usage::record(&conn, &event) {
        let _ = ctx
            .app
            .emit("usage.updated", json!({ "runId": run.run_id }));
        let _ = ctx.app.emit(
            "run.delta",
            json!({
                "runId": run.run_id,
                "kind": "usage",
                "usage": {
                    "inputTokens": input,
                    "outputTokens": output,
                    "cacheReadTokens": cache_read,
                    "cacheWriteTokens": cache_write,
                    "costMicros": cost
                }
            }),
        );
        let scope = BudgetScope {
            project_id: &run.project_id,
            team_id: &run.team_id,
            agent_id: &run.agent_id,
            task_id: &run.task_id,
            run_id: &run.run_id,
        };
        budget::enforce(&ctx.app, &conn, &scope, &ctx.registry, run.notify);
    }
    let _ = log_event(
        &conn,
        Some(&run.project_id),
        Some(&run.task_id),
        Some(&run.run_id),
        "llm.proxy",
        Some("llm-proxy"),
        json!({
            "model": model,
            "inputTokens": input,
            "outputTokens": output,
            "costMicros": crate::usage::compute_cost_micros(
                &conn,
                &usage::provider_for(&model),
                &model,
                input,
                output,
                cache_read,
                cache_write,
            )
            .unwrap_or(0)
        }),
    );
}

fn json_status(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

/// Anthropic / OpenAI 都能识别的错误结构，避免 SDK 当成可重试错误反复冲击预算。
fn budget_error(message: &str) -> Response {
    (
        StatusCode::PAYMENT_REQUIRED,
        Json(json!({
            "type": "error",
            "error": { "type": "invalid_request_error", "message": message }
        })),
    )
        .into_response()
}

/// 增量解析 SSE / JSON 中的 usage。Anthropic 的 message_start 带输入与缓存，
/// message_delta 带累计输出；OpenAI 在最后一个 chunk 里给出 prompt/completion。
#[derive(Debug, Clone, Default)]
struct UsageScan {
    buf: Vec<u8>,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    model: String,
}

impl UsageScan {
    fn total(&self) -> i64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    fn feed(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
        while let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let text = String::from_utf8_lossy(&line);
            self.line(text.trim());
        }
        // 非 SSE 的超大响应不该无限累积，保留尾部即可。
        if self.buf.len() > 256 * 1024 {
            self.buf.drain(..self.buf.len() - 16 * 1024);
        }
    }

    fn finish(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        let text = String::from_utf8_lossy(&self.buf).to_string();
        self.buf.clear();
        for line in text.lines() {
            self.line(line.trim());
        }
    }

    fn line(&mut self, line: &str) {
        let data = line.strip_prefix("data:").map(str::trim).unwrap_or(line);
        if data.is_empty() || data == "[DONE]" {
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };
        if let Some(model) = value
            .get("message")
            .and_then(|m| m.get("model"))
            .and_then(|m| m.as_str())
            .or_else(|| value.get("model").and_then(|m| m.as_str()))
        {
            self.model = model.to_string();
        }
        let usage = value
            .get("message")
            .and_then(|m| m.get("usage"))
            .or_else(|| value.get("usage"));
        let Some(usage) = usage else { return };
        if let Some(input) = usage
            .get("input_tokens")
            .or_else(|| usage.get("prompt_tokens"))
            .and_then(|v| v.as_i64())
        {
            self.input = self.input.max(input);
        }
        if let Some(output) = usage
            .get("output_tokens")
            .or_else(|| usage.get("completion_tokens"))
            .and_then(|v| v.as_i64())
        {
            self.output = self.output.max(output);
        }
        if let Some(read) = usage
            .get("cache_read_input_tokens")
            .or_else(|| {
                usage
                    .get("prompt_tokens_details")
                    .and_then(|d| d.get("cached_tokens"))
            })
            .and_then(|v| v.as_i64())
        {
            self.cache_read = self.cache_read.max(read);
        }
        if let Some(write) = usage
            .get("cache_creation_input_tokens")
            .and_then(|v| v.as_i64())
        {
            self.cache_write = self.cache_write.max(write);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::Headroom;

    #[test]
    fn parses_anthropic_stream_usage() {
        let mut scan = UsageScan::default();
        scan.feed(
            br#"data: {"type":"message_start","message":{"model":"claude-sonnet-4","usage":{"input_tokens":120,"cache_read_input_tokens":30,"cache_creation_input_tokens":10,"output_tokens":1}}}
"#,
        );
        scan.feed(
            br#"data: {"type":"message_delta","usage":{"output_tokens":220}}
"#,
        );
        scan.finish();
        assert_eq!(
            (scan.input, scan.output, scan.cache_read, scan.cache_write),
            (120, 220, 30, 10)
        );
        assert_eq!(scan.model, "claude-sonnet-4");
    }

    #[test]
    fn parses_openai_body_usage() {
        let mut scan = UsageScan::default();
        scan.feed(
            br#"{"model":"gpt-4o","usage":{"prompt_tokens":80,"completion_tokens":40,"prompt_tokens_details":{"cached_tokens":16}}}"#,
        );
        scan.finish();
        assert_eq!((scan.input, scan.output, scan.cache_read), (80, 40, 16));
    }

    #[test]
    fn clamps_output_to_token_and_usd_headroom() {
        let tokens = Headroom {
            usd_micros: None,
            tokens: Some(1_000),
        };
        assert_eq!(clamp_output(tokens, 200, 4_000, 15_000_000), 800);

        let usd = Headroom {
            usd_micros: Some(1_000_000),
            tokens: None,
        };
        let allowed = clamp_output(usd, 1_000, 100_000, 15_000_000);
        assert!(allowed > 60_000 && allowed < 70_000);

        let zero = Headroom {
            usd_micros: Some(0),
            tokens: None,
        };
        assert_eq!(clamp_output(zero, 10, 100, 15_000_000), 0);
    }

    #[test]
    fn joins_upstream_without_duplicating_v1() {
        assert_eq!(
            join_upstream("https://api.anthropic.com", "v1/messages", None),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            join_upstream("https://gw.example.com/v1", "v1/chat/completions", None),
            "https://gw.example.com/v1/chat/completions"
        );
        assert_eq!(
            join_upstream("https://gw.example.com", "v1/models", Some("limit=1")),
            "https://gw.example.com/v1/models?limit=1"
        );
    }

    #[test]
    fn recognises_llm_paths_only() {
        assert!(is_llm_path("v1/messages"));
        assert!(is_llm_path("v1/chat/completions"));
        assert!(!is_llm_path("v1/messages/count_tokens"));
        assert!(!is_llm_path("v1/models"));
    }
}
