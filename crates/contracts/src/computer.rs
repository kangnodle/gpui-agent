use crate::{ControlCommand,
            ControlFrame};
use serde::{Deserialize,
            Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ComputerStatus {
    #[serde(default)]
    pub automatic: bool,
    pub available: bool,
    pub manual_enabled: bool,
    pub job_id: Option<i64>,
    pub execution_bound: bool,
    pub expires_in_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground: Option<ComputerForeground>,
}

/// Identifies a local Windows process without trusting a possibly remote PID.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComputerForeground {
    pub process_id: u32,
    pub started_at: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerGrantRequest {
    pub job_id: i64,
    pub enabled: bool,
}
impl ComputerGrantRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.job_id <= 0 {
            return Err("Computer grant requires a positive job ID");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputerCommand {
    Capture {},
    Act { revision: String, action: ControlCommand },
    Close {},
}
impl ComputerCommand {
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Self::Act {
            revision,
            action,
        } = self
        {
            if revision.is_empty() || revision.len() > 80 {
                return Err("Use the latest computer capture revision");
            }
            if !matches!(
                action,
                ControlCommand::Click { .. } | ControlCommand::Text { .. } | ControlCommand::Key { .. } | ControlCommand::Scroll { .. }
            ) {
                return Err("Unsupported computer action");
            }
            action.validate()?;
        }
        Ok(())
    }
}

pub struct ComputerLease {
    pub job_id: i64,
    state: gpui_agent_domain::ComputerLeaseState,
}
impl ComputerLease {
    pub fn new(job_id: i64) -> Self {
        Self {
            job_id,
            state: gpui_agent_domain::ComputerLeaseState::granted(job_id),
        }
    }

    pub fn state(&self) -> &gpui_agent_domain::ComputerLeaseState { &self.state }

    pub fn authorize(&self, job_id: i64, execution_id: u64) -> Result<(), &'static str> { self.state.authorize(job_id, execution_id) }

    pub fn begin(&mut self, execution_id: u64) {
        if let Ok(next) = gpui_agent_domain::transition_computer_lease(
            self.state.clone(),
            gpui_agent_domain::ComputerLeaseInput::Bind {
                execution_id,
            },
        ) {
            self.state = next;
        }
    }

    pub fn observe(&mut self, revision: String, width: u32, height: u32) {
        let observation = gpui_agent_domain::ScreenObservation {
            revision,
            width,
            height,
        };
        if let Ok(next) = gpui_agent_domain::transition_computer_lease(self.state.clone(), gpui_agent_domain::ComputerLeaseInput::Observe(observation)) {
            self.state = next;
        }
    }

    pub fn consume(&mut self, revision: &str, action: &ControlCommand) -> Result<(), &'static str> {
        let target = match action {
            | ControlCommand::Click {
                x,
                y,
            } => gpui_agent_domain::ComputerInputTarget::Click {
                x: *x,
                y: *y,
            },
            | _ => gpui_agent_domain::ComputerInputTarget::Other,
        };
        let next = gpui_agent_domain::transition_computer_lease(
            self.state.clone(),
            gpui_agent_domain::ComputerLeaseInput::Consume {
                revision: revision.to_owned(),
                target,
            },
        )?;
        self.state = next;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ComputerFrame {
    pub revision: String,
    #[serde(flatten)]
    pub frame: ControlFrame,
}
