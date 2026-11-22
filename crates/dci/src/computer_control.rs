use gpui_agent_contracts::{ComputerInputTarget,
                           ComputerLeaseInput,
                           ComputerLeaseState,
                           ControlCommand,
                           ScreenObservation,
                           transition_computer_lease};

pub trait ComputerLeaseRole {
    fn authorize_session(&self, job_id: i64, execution_id: u64) -> Result<(), &'static str>;
    fn apply_input(self, input: ComputerLeaseInput) -> Result<ComputerLeaseState, &'static str>;
}

impl ComputerLeaseRole for ComputerLeaseState {
    fn authorize_session(&self, job_id: i64, execution_id: u64) -> Result<(), &'static str> { self.authorize(job_id, execution_id) }

    fn apply_input(self, input: ComputerLeaseInput) -> Result<ComputerLeaseState, &'static str> { transition_computer_lease(self, input) }
}

pub struct ComputerLeaseContext<L: ComputerLeaseRole> {
    lease: L,
}

impl<L: ComputerLeaseRole> ComputerLeaseContext<L> {
    pub fn new(lease: L) -> Self {
        Self {
            lease,
        }
    }

    pub fn authorize(&self, job_id: i64, execution_id: u64) -> Result<(), &'static str> { self.lease.authorize_session(job_id, execution_id) }

    pub fn bind(self, execution_id: u64) -> Result<ComputerLeaseState, &'static str> {
        self.lease.apply_input(ComputerLeaseInput::Bind {
            execution_id,
        })
    }

    pub fn observe(self, revision: String, width: u32, height: u32) -> Result<ComputerLeaseState, &'static str> {
        self.lease.apply_input(ComputerLeaseInput::Observe(ScreenObservation {
            revision,
            width,
            height,
        }))
    }

    pub fn consume_command(self, revision: &str, command: &ControlCommand) -> Result<ComputerLeaseState, &'static str> {
        let target = match command {
            | ControlCommand::Click {
                x,
                y,
            } => ComputerInputTarget::Click {
                x: *x,
                y: *y,
            },
            | _ => ComputerInputTarget::Other,
        };
        self.lease.apply_input(ComputerLeaseInput::Consume {
            revision: revision.to_owned(),
            target,
        })
    }
}
