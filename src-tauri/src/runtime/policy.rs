/// Terminal status for a single run attempt.
///
/// `Starting` and `Running` are persistence states owned by the store, not
/// outcomes. Keeping the enum terminal-only makes the precedence rules explicit
/// and keeps the policy free of database and process concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunStatus {
    Completed,
    Failed,
    Cancelled,
    PausedBudget,
}

impl RunStatus {
    pub(crate) fn as_db_str(self) -> &'static str {
        match self {
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
            RunStatus::PausedBudget => "paused_budget",
        }
    }
}

pub(crate) struct OutcomeInputs<'a> {
    pub(crate) budget_blocked: bool,
    pub(crate) hook_bypass: bool,
    pub(crate) hook_strict: bool,
    pub(crate) cancelled: bool,
    pub(crate) exit_code: i32,
    pub(crate) reported_error: Option<&'a str>,
}

/// Decide the terminal status and user-facing error for one run.
///
/// Precedence matches the original runtime behaviour exactly:
/// budget hard stop, strict hook bypass, user cancellation, clean exit, failure.
pub(crate) fn evaluate(inputs: OutcomeInputs<'_>) -> (RunStatus, Option<String>) {
    if inputs.budget_blocked {
        return (
            RunStatus::PausedBudget,
            Some("预算达到上限，运行已暂停".to_string()),
        );
    }
    if inputs.hook_bypass && inputs.hook_strict {
        return (
            RunStatus::Failed,
            Some("检测到未经审批网关的工具调用，严格模式判定失败".to_string()),
        );
    }
    if inputs.cancelled {
        return (RunStatus::Cancelled, None);
    }
    if inputs.exit_code == 0 && inputs.reported_error.is_none() {
        return (RunStatus::Completed, None);
    }
    (
        RunStatus::Failed,
        inputs
            .reported_error
            .map(str::to_string)
            .or_else(|| Some(format!("Agent 退出码 {}", inputs.exit_code))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evaluate_all(
        budget_blocked: bool,
        hook_bypass: bool,
        hook_strict: bool,
        cancelled: bool,
        exit_code: i32,
        reported_error: Option<&str>,
    ) -> (RunStatus, Option<String>) {
        evaluate(OutcomeInputs {
            budget_blocked,
            hook_bypass,
            hook_strict,
            cancelled,
            exit_code,
            reported_error,
        })
    }

    #[test]
    fn clean_exit_completes() {
        assert_eq!(
            evaluate_all(false, false, true, false, 0, None),
            (RunStatus::Completed, None)
        );
    }

    #[test]
    fn budget_wins_over_cancel() {
        assert_eq!(
            evaluate_all(true, false, true, true, 0, None),
            (
                RunStatus::PausedBudget,
                Some("预算达到上限，运行已暂停".to_string())
            )
        );
    }

    #[test]
    fn budget_wins_over_strict_hook_bypass() {
        assert_eq!(
            evaluate_all(true, true, true, false, 0, None),
            (
                RunStatus::PausedBudget,
                Some("预算达到上限，运行已暂停".to_string())
            )
        );
    }

    #[test]
    fn strict_hook_bypass_wins_over_cancel() {
        assert_eq!(
            evaluate_all(false, true, true, true, 0, None),
            (
                RunStatus::Failed,
                Some("检测到未经审批网关的工具调用，严格模式判定失败".to_string())
            )
        );
    }

    #[test]
    fn non_strict_hook_bypass_still_cancels() {
        assert_eq!(
            evaluate_all(false, true, false, true, 0, None),
            (RunStatus::Cancelled, None)
        );
    }

    #[test]
    fn cancel_wins_over_clean_exit() {
        assert_eq!(
            evaluate_all(false, false, true, true, 0, None),
            (RunStatus::Cancelled, None)
        );
    }

    #[test]
    fn reported_error_fails_even_on_zero_exit() {
        assert_eq!(
            evaluate_all(false, false, true, false, 0, Some("adapter error")),
            (RunStatus::Failed, Some("adapter error".to_string()))
        );
    }

    #[test]
    fn non_zero_exit_without_error_uses_exit_code() {
        assert_eq!(
            evaluate_all(false, false, true, false, 7, None),
            (RunStatus::Failed, Some("Agent 退出码 7".to_string()))
        );
    }

    #[test]
    fn missing_exit_code_is_failure() {
        assert_eq!(
            evaluate_all(false, false, true, false, -1, None),
            (RunStatus::Failed, Some("Agent 退出码 -1".to_string()))
        );
    }
}
