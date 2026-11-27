use serde_json::Value;
use tauri::{AppHandle, Emitter};

/// Narrow event seam for run-level UI notifications.
///
/// Production forwards to Tauri; tests record payloads without constructing an
/// `AppHandle`.
pub(crate) trait RunEvents: Send + Sync {
    fn emit(&self, event: &str, payload: Value);
}

#[derive(Clone)]
pub(crate) struct TauriEvents {
    app: AppHandle,
}

impl TauriEvents {
    pub(crate) fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl RunEvents for TauriEvents {
    fn emit(&self, event: &str, payload: Value) {
        let _ = self.app.emit(event, payload);
    }
}

#[cfg(test)]
pub(crate) struct RecordingEvents {
    inner: std::sync::Mutex<Vec<(String, Value)>>,
}

#[cfg(test)]
impl RecordingEvents {
    pub(crate) fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn events(&self) -> Vec<(String, Value)> {
        self.inner
            .lock()
            .map(|events| events.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
impl RunEvents for RecordingEvents {
    fn emit(&self, event: &str, payload: Value) {
        if let Ok(mut events) = self.inner.lock() {
            events.push((event.to_string(), payload));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recording_events_capture_payloads() {
        let events = RecordingEvents::new();
        events.emit("run.status", json!({ "runId": "r1", "status": "running" }));

        let recorded = events.events();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "run.status");
        assert_eq!(recorded[0].1["status"], "running");
    }
}
