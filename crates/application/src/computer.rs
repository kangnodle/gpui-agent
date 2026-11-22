use crate::{ApplicationResult,
            Jobs,
            ports::{ComputerSession,
                    HostControl}};
use gpui_agent_contracts::*;
use std::sync::Arc;

fn active(job: &Job) -> bool { matches!(job.state, JobState::Running | JobState::Waiting) && !job.cancel_requested }

pub async fn computer_status(backend: &dyn HostControl) -> ApplicationResult<ComputerStatus> { Ok(backend.computer_status().await?) }

impl Jobs {
    pub async fn grant_computer(&self, backend: &dyn HostControl, grant: ComputerGrantRequest) -> ApplicationResult<()> {
        grant.validate()?;
        let id = grant.job_id;
        if grant.enabled && !active(&self.job(id).await?) {
            return Err("Computer grants require an active job".into());
        }
        let repository = self.0.repository.clone();
        backend
            .grant_computer(grant, Arc::new(move || repository.job(id).is_ok_and(|job| active(&job))))
            .await?;
        Ok(())
    }

    pub async fn control_computer(
        &self,
        backend: &dyn HostControl,
        caller: TaskSession,
        mut session: ComputerSession,
        command: ComputerCommand,
    ) -> ApplicationResult<Option<ComputerFrame>> {
        command.validate()?;
        let id = session.job_id;
        let job = self.job(id).await?;
        if !active(&job) || !caller.scope.allows(&job.scope) || caller.conversation_id != job.conversation_id {
            return Err("Computer use requires its own active job".into());
        }
        let repository = self.0.repository.clone();
        let credential_active = session.active;
        session.active = Arc::new(move || {
            credential_active()
                && repository
                    .job(id)
                    .is_ok_and(|job| active(&job) && caller.scope.allows(&job.scope) && caller.conversation_id == job.conversation_id)
        });
        let reply = backend.computer(session, command).await?;
        if let Some(capture) = &reply {
            capture.frame.image.validate()?;
            if !matches!(capture.frame.image, Attachment::Image { .. }) || capture.frame.width == 0 || capture.frame.height == 0 {
                return Err("Invalid computer capture".into());
            }
        }
        Ok(reply)
    }
}
