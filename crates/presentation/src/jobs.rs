use gpui_agent_contracts::{Job,
                           JobState};
use std::collections::{HashMap,
                       HashSet};

/// Notifications follow server jobs, independently of the visible project.
#[derive(Default)]
pub struct JobActivity {
    initialized: bool,
    states: HashMap<i64, JobState>,
    unread: HashSet<i64>,
    changed: HashSet<Option<i64>>,
}

impl JobActivity {
    pub fn observe(&mut self, job: &Job) {
        let previous = self.states.insert(job.id, job.state);
        if job.state.terminal() && previous.is_none_or(|state| !state.terminal()) && (self.initialized || previous.is_some()) {
            self.unread.insert(job.id);
            self.changed.insert(job.scope.workspace_id);
        }
    }

    pub fn catalog(&mut self, jobs: &[Job]) {
        for job in jobs {
            self.observe(job);
        }
        self.initialized = true;
        let present: HashSet<_> = jobs.iter().map(|job| job.id).collect();
        self.states.retain(|id, _| present.contains(id));
        self.unread.retain(|id| present.contains(id));
    }

    pub fn unread(&self, id: i64) -> bool { self.unread.contains(&id) }

    pub fn read(&mut self, id: i64) { self.unread.remove(&id); }

    pub fn take_changed(&mut self, workspace: Option<i64>) -> bool { self.changed.remove(&workspace) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_agent_contracts::{JobScope,
                               Provider};

    fn job(id: i64, workspace: i64, state: JobState) -> Job {
        Job {
            id,
            state,
            scope: JobScope {
                provider: Provider::Codex,
                account_id: "default".into(),
                workspace_id: Some(workspace),
                workspace_path: format!("/{workspace}"),
            },
            conversation_id: None,
            operation_id: None,
            pending_request_id: None,
            last_sequence: 0,
            preview: String::new(),
            cancel_requested: false,
            error: None,
            created_at: 0,
        }
    }

    #[test]
    fn notifications_are_scoped_and_replayed_completion_is_only_reported_once() {
        let mut activity = JobActivity::default();
        activity.catalog(&[job(1, 10, JobState::Completed), job(2, 20, JobState::Running)]);
        assert!(!activity.unread(1));
        activity.observe(&job(2, 20, JobState::Completed));
        assert!(activity.unread(2));
        assert!(!activity.take_changed(Some(10)));
        assert!(activity.take_changed(Some(20)));
        activity.read(2);
        activity.catalog(&[job(1, 10, JobState::Completed), job(2, 20, JobState::Completed)]);
        assert!(!activity.unread(2));
        assert!(!activity.take_changed(Some(20)));
        activity.observe(&job(3, 10, JobState::Failed));
        assert!(activity.unread(3));
        assert!(activity.take_changed(Some(10)));
    }
}
