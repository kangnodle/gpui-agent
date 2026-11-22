//! Deterministic text and predicates shared by clients' views.
use gpui_agent_contracts::{Job,
                           JobState};

pub fn state_label(state: JobState) -> &'static str {
    match state {
        | JobState::Queued => "대기 중",
        | JobState::Running => "실행 중",
        | JobState::Waiting => "사용자 확인 대기",
        | JobState::Completed => "완료",
        | JobState::Failed => "실패",
        | JobState::Cancelled => "취소됨",
        | JobState::Interrupted => "중단됨",
    }
}

/// A job that can still act on the host: running or waiting, not being
/// cancelled.
pub fn is_active(job: &Job) -> bool { matches!(job.state, JobState::Running | JobState::Waiting) && !job.cancel_requested }

/// Wraps text in a Markdown fence longer than any backtick run inside it.
pub fn plain(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}\n{text}\n{fence}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fence_outgrows_backtick_runs_in_the_text() {
        assert_eq!(plain("a"), "```\na\n```");
        assert_eq!(plain("x ```` y"), "`````\nx ```` y\n`````");
    }

    #[test]
    fn states_have_distinct_labels() {
        assert_eq!(state_label(JobState::Waiting), "사용자 확인 대기");
    }
}
