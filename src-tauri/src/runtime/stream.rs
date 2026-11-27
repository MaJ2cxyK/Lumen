use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MessageOrigin {
    Claude,
    Stdout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UsageDelta {
    pub(crate) model: String,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) cache_read_tokens: i64,
    pub(crate) cache_write_tokens: i64,
    pub(crate) source: &'static str,
    pub(crate) dedupe_key: String,
    pub(crate) report_updated: bool,
    pub(crate) report_alert: bool,
    pub(crate) report_delta: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RunEffect {
    SessionObserved {
        session_id: String,
    },
    ChatMessage {
        model: Option<String>,
        text: String,
        origin: MessageOrigin,
    },
    ToolCall {
        tool_id: String,
        tool_name: String,
        input: Value,
        gated: bool,
    },
    ToolResult {
        tool_id: String,
        output: Value,
    },
    Usage(UsageDelta),
    RawLine {
        line: String,
    },
    Result {
        final_text: Option<String>,
        session_id: Option<String>,
        error: Option<String>,
        fallback_usage: Option<UsageDelta>,
    },
}

pub(crate) enum StreamInterpreter {
    Claude(ClaudeParser),
    Generic(GenericParser),
}

impl StreamInterpreter {
    pub(crate) fn new(
        adapter: &str,
        run_id: &str,
        model: &str,
        prompt: &str,
        use_proxy: bool,
    ) -> Self {
        if adapter == "claude-code" {
            Self::Claude(ClaudeParser {
                run_id: run_id.to_string(),
                model: model.to_string(),
                use_proxy,
            })
        } else {
            Self::Generic(GenericParser {
                run_id: run_id.to_string(),
                model: model.to_string(),
                prompt_chars: prompt.chars().count(),
                use_proxy,
                output: String::new(),
                bytes: 0,
                input_recorded: false,
                output_recorded: 0,
            })
        }
    }

    /// Text captured from a non-Claude adapter, used only for proxy fallback.
    pub(crate) fn text_buffer(&self) -> &str {
        match self {
            Self::Claude(_) => "",
            Self::Generic(parser) => &parser.output,
        }
    }

    pub(crate) fn feed(&mut self, line: &str) -> Vec<RunEffect> {
        match self {
            Self::Claude(parser) => parser.feed(line),
            Self::Generic(parser) => parser.feed(line),
        }
    }

    pub(crate) fn finish(&mut self) -> Vec<RunEffect> {
        match self {
            Self::Claude(_) => Vec::new(),
            Self::Generic(parser) => parser.finish(),
        }
    }
}

pub(crate) struct ClaudeParser {
    run_id: String,
    model: String,
    use_proxy: bool,
}

impl ClaudeParser {
    fn feed(&mut self, line: &str) -> Vec<RunEffect> {
        if line.trim().is_empty() {
            return Vec::new();
        }
        let parsed: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => return Vec::new(),
        };
        let kind = parsed
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        match kind {
            "system" => parsed
                .get("session_id")
                .and_then(|value| value.as_str())
                .map(|session_id| RunEffect::SessionObserved {
                    session_id: session_id.to_string(),
                })
                .into_iter()
                .collect(),
            "assistant" => self.assistant_effects(&parsed, line),
            "user" => self.tool_result_effects(&parsed),
            "result" => vec![self.result_effect(&parsed)],
            _ => Vec::new(),
        }
    }

    fn assistant_effects(&self, parsed: &Value, line: &str) -> Vec<RunEffect> {
        let Some(message) = parsed.get("message") else {
            return Vec::new();
        };
        let model = message
            .get("model")
            .and_then(|value| value.as_str())
            .unwrap_or(&self.model)
            .to_string();
        let content = message.get("content").cloned().unwrap_or(Value::Null);
        let text = text_of_content(&content);
        let mut effects = Vec::new();
        if !text.trim().is_empty() {
            effects.push(RunEffect::ChatMessage {
                model: Some(model.clone()),
                text,
                origin: MessageOrigin::Claude,
            });
        }
        if let Some(blocks) = content.as_array() {
            for block in blocks {
                if block.get("type").and_then(|value| value.as_str()) != Some("tool_use") {
                    continue;
                }
                let tool_name = block
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or("tool");
                let tool_id = block
                    .get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                let input = block.get("input").cloned().unwrap_or(Value::Null);
                effects.push(RunEffect::ToolCall {
                    tool_id: tool_id.to_string(),
                    tool_name: tool_name.to_string(),
                    input,
                    gated: super::gate_tool_matched(tool_name),
                });
            }
        }
        if !self.use_proxy {
            if let Some(usage) = message.get("usage") {
                let message_id = message
                    .get("id")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| super::hash_line(line));
                effects.push(RunEffect::Usage(UsageDelta {
                    model,
                    input_tokens: usage
                        .get("input_tokens")
                        .and_then(|value| value.as_i64())
                        .unwrap_or(0),
                    output_tokens: usage
                        .get("output_tokens")
                        .and_then(|value| value.as_i64())
                        .unwrap_or(0),
                    cache_read_tokens: usage
                        .get("cache_read_input_tokens")
                        .and_then(|value| value.as_i64())
                        .unwrap_or(0),
                    cache_write_tokens: usage
                        .get("cache_creation_input_tokens")
                        .and_then(|value| value.as_i64())
                        .unwrap_or(0),
                    source: "stream",
                    dedupe_key: format!("claude:{}:{}", self.run_id, message_id),
                    report_updated: true,
                    report_alert: true,
                    report_delta: true,
                }));
            }
        }
        effects
    }

    fn tool_result_effects(&self, parsed: &Value) -> Vec<RunEffect> {
        let Some(blocks) = parsed
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(|content| content.as_array())
        else {
            return Vec::new();
        };
        blocks
            .iter()
            .filter(|block| {
                block.get("type").and_then(|value| value.as_str()) == Some("tool_result")
            })
            .map(|block| RunEffect::ToolResult {
                tool_id: block
                    .get("tool_use_id")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string(),
                output: block.get("content").cloned().unwrap_or(Value::Null),
            })
            .collect()
    }

    fn result_effect(&self, parsed: &Value) -> RunEffect {
        let final_text = parsed
            .get("result")
            .and_then(|value| value.as_str())
            .filter(|text| !text.trim().is_empty())
            .map(str::to_string);
        let session_id = parsed
            .get("session_id")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let is_error = parsed
            .get("is_error")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let error = is_error.then(|| {
            parsed
                .get("result")
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| "Agent 返回错误".into())
        });
        let fallback_usage = if !self.use_proxy {
            parsed.get("usage").map(|usage| UsageDelta {
                model: parsed
                    .get("modelUsage")
                    .and_then(|value| value.as_object())
                    .and_then(|models| models.keys().next().cloned())
                    .unwrap_or_else(|| self.model.clone()),
                input_tokens: usage
                    .get("input_tokens")
                    .and_then(|value| value.as_i64())
                    .unwrap_or(0),
                output_tokens: usage
                    .get("output_tokens")
                    .and_then(|value| value.as_i64())
                    .unwrap_or(0),
                cache_read_tokens: usage
                    .get("cache_read_input_tokens")
                    .and_then(|value| value.as_i64())
                    .unwrap_or(0),
                cache_write_tokens: usage
                    .get("cache_creation_input_tokens")
                    .and_then(|value| value.as_i64())
                    .unwrap_or(0),
                source: "stream",
                dedupe_key: format!("claude:{}:result", self.run_id),
                report_updated: false,
                report_alert: false,
                report_delta: false,
            })
        } else {
            None
        };
        RunEffect::Result {
            final_text,
            session_id,
            error,
            fallback_usage,
        }
    }
}

pub(crate) struct GenericParser {
    run_id: String,
    model: String,
    prompt_chars: usize,
    use_proxy: bool,
    output: String,
    bytes: usize,
    input_recorded: bool,
    output_recorded: i64,
}

impl GenericParser {
    fn feed(&mut self, line: &str) -> Vec<RunEffect> {
        self.output.push_str(line);
        self.output.push('\n');
        let mut effects = vec![RunEffect::RawLine {
            line: line.to_string(),
        }];
        if self.use_proxy {
            return effects;
        }
        self.bytes += line.len() + 1;
        while self.bytes as i64 - self.output_recorded * 4 >= 4096 {
            if !self.input_recorded {
                effects.push(RunEffect::Usage(self.input_estimate()));
                self.input_recorded = true;
            }
            effects.push(RunEffect::Usage(self.output_estimate(1024)));
            self.output_recorded += 1024;
        }
        effects
    }

    fn finish(&mut self) -> Vec<RunEffect> {
        let mut effects = Vec::new();
        if !self.output.trim().is_empty() {
            effects.push(RunEffect::ChatMessage {
                model: None,
                text: self.output.trim().to_string(),
                origin: MessageOrigin::Stdout,
            });
        }
        if self.use_proxy {
            return effects;
        }
        if !self.input_recorded {
            effects.push(RunEffect::Usage(self.input_estimate()));
        }
        let total = ((self.output.chars().count() as f64) / 4.0).ceil().max(1.0) as i64;
        let delta = (total - self.output_recorded).max(0);
        if delta > 0 {
            effects.push(RunEffect::Usage(UsageDelta {
                model: self.model.clone(),
                input_tokens: 0,
                output_tokens: delta,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                source: "estimate",
                dedupe_key: format!("estimate:{}:out:final", self.run_id),
                report_updated: true,
                report_alert: true,
                report_delta: false,
            }));
        }
        effects
    }

    fn input_estimate(&self) -> UsageDelta {
        UsageDelta {
            model: self.model.clone(),
            input_tokens: ((self.prompt_chars as f64) / 4.0).ceil().max(1.0) as i64,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            source: "estimate",
            dedupe_key: format!("estimate:{}:input", self.run_id),
            report_updated: false,
            report_alert: false,
            report_delta: false,
        }
    }

    fn output_estimate(&self, tokens: i64) -> UsageDelta {
        UsageDelta {
            model: self.model.clone(),
            input_tokens: 0,
            output_tokens: tokens,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            source: "estimate",
            dedupe_key: format!("estimate:{}:out:{}", self.run_id, self.output_recorded),
            report_updated: true,
            report_alert: false,
            report_delta: false,
        }
    }
}

fn text_of_content(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    let mut parts = Vec::new();
    if let Some(blocks) = content.as_array() {
        for block in blocks {
            if block.get("type").and_then(|value| value.as_str()) == Some("text") {
                if let Some(text) = block.get("text").and_then(|value| value.as_str()) {
                    parts.push(text.to_string());
                }
            }
        }
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn claude(line: Value, run_id: &str, model: &str, use_proxy: bool) -> Vec<RunEffect> {
        let mut parser = StreamInterpreter::new("claude-code", run_id, model, "prompt", use_proxy);
        parser.feed(&line.to_string())
    }

    #[test]
    fn system_line_observes_session() {
        let effects = claude(
            json!({ "type": "system", "session_id": "sess-1" }),
            "r1",
            "claude",
            false,
        );
        assert_eq!(
            effects,
            vec![RunEffect::SessionObserved {
                session_id: "sess-1".into()
            }]
        );
    }

    #[test]
    fn assistant_text_and_blank_text() {
        let effects = claude(
            json!({
                "type": "assistant",
                "message": { "model": "m1", "content": [{ "type": "text", "text": "hi" }] }
            }),
            "r1",
            "claude",
            false,
        );
        assert_eq!(
            effects,
            vec![RunEffect::ChatMessage {
                model: Some("m1".into()),
                text: "hi".into(),
                origin: MessageOrigin::Claude,
            }]
        );
        assert!(claude(
            json!({
                "type": "assistant",
                "message": { "content": [{ "type": "text", "text": "  " }] }
            }),
            "r1",
            "claude",
            false,
        )
        .is_empty());
    }

    #[test]
    fn tool_use_marks_gated_only_for_managed_tools() {
        let effects = claude(
            json!({
                "type": "assistant",
                "message": {
                    "content": [
                        { "type": "tool_use", "id": "t1", "name": "Bash", "input": { "cmd": "ls" } },
                        { "type": "tool_use", "id": "t2", "name": "Read", "input": {} }
                    ]
                }
            }),
            "r1",
            "claude",
            false,
        );
        let calls: Vec<_> = effects
            .into_iter()
            .filter_map(|effect| match effect {
                RunEffect::ToolCall { tool_id, gated, .. } => Some((tool_id, gated)),
                _ => None,
            })
            .collect();
        assert_eq!(
            calls,
            vec![("t1".to_string(), true), ("t2".to_string(), false)]
        );
    }

    #[test]
    fn tool_result_emits_completion() {
        let effects = claude(
            json!({
                "type": "user",
                "message": {
                    "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": "ok" }]
                }
            }),
            "r1",
            "claude",
            false,
        );
        assert_eq!(
            effects,
            vec![RunEffect::ToolResult {
                tool_id: "t1".into(),
                output: json!("ok")
            }]
        );
    }

    #[test]
    fn result_carries_text_session_error_and_usage() {
        let effects = claude(
            json!({
                "type": "result",
                "result": "done",
                "session_id": "sess-2",
                "is_error": true,
                "usage": { "input_tokens": 10, "output_tokens": 20 },
                "modelUsage": { "m2": {} }
            }),
            "r1",
            "claude",
            false,
        );
        match &effects[0] {
            RunEffect::Result {
                final_text,
                session_id,
                error,
                fallback_usage,
            } => {
                assert_eq!(final_text.as_deref(), Some("done"));
                assert_eq!(session_id.as_deref(), Some("sess-2"));
                assert_eq!(error.as_deref(), Some("done"));
                let usage = fallback_usage.as_ref().unwrap();
                assert_eq!(usage.model, "m2");
                assert_eq!(usage.input_tokens, 10);
                assert_eq!(usage.output_tokens, 20);
                assert_eq!(usage.dedupe_key, "claude:r1:result");
            }
            _ => panic!("expected result effect"),
        }
    }

    #[test]
    fn multiple_results_last_wins_when_applied() {
        let mut parser = StreamInterpreter::new("claude-code", "r1", "claude", "prompt", false);
        let first = parser.feed(&json!({ "type": "result", "result": "first" }).to_string());
        let second = parser.feed(&json!({ "type": "result", "result": "second" }).to_string());
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
    }

    #[test]
    fn malformed_json_is_ignored() {
        assert!(claude(json!(null), "r1", "claude", false).is_empty());
        let mut parser = StreamInterpreter::new("claude-code", "r1", "claude", "prompt", false);
        assert!(parser.feed("{not json").is_empty());
    }

    #[test]
    fn assistant_usage_uses_message_id_as_dedupe_key() {
        let effects = claude(
            json!({
                "type": "assistant",
                "message": {
                    "id": "msg-1",
                    "content": "hello",
                    "usage": { "input_tokens": 3, "output_tokens": 4 }
                }
            }),
            "r1",
            "claude",
            false,
        );
        match &effects[1] {
            RunEffect::Usage(usage) => assert_eq!(usage.dedupe_key, "claude:r1:msg-1"),
            _ => panic!("expected usage effect"),
        }
    }

    #[test]
    fn generic_estimates_every_four_kilobytes() {
        let mut parser = StreamInterpreter::new("generic", "r1", "m", "prompt", false);
        let line = "x".repeat(4096);
        let effects = parser.feed(&line);
        let usage: Vec<_> = effects
            .into_iter()
            .filter_map(|effect| match effect {
                RunEffect::Usage(delta) => Some(delta),
                _ => None,
            })
            .collect();
        assert_eq!(usage.len(), 2);
        assert_eq!(usage[0].source, "estimate");
        assert_eq!(usage[1].output_tokens, 1024);
    }

    #[test]
    fn proxy_mode_never_estimates() {
        let mut parser = StreamInterpreter::new("generic", "r1", "m", "prompt", true);
        let effects = parser.feed(&"x".repeat(4096));
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], RunEffect::RawLine { .. }));
        assert!(parser
            .finish()
            .iter()
            .all(|effect| { matches!(effect, RunEffect::ChatMessage { .. }) }));
    }

    #[test]
    fn generic_finish_emits_message_and_remaining_usage() {
        let mut parser = StreamInterpreter::new("generic", "r1", "m", "prompt", false);
        parser.feed("hi");
        let effects = parser.finish();
        assert!(matches!(effects[0], RunEffect::ChatMessage { .. }));
        let usage: Vec<_> = effects
            .into_iter()
            .filter_map(|effect| match effect {
                RunEffect::Usage(delta) => Some(delta),
                _ => None,
            })
            .collect();
        assert_eq!(usage.len(), 2);
    }
}
