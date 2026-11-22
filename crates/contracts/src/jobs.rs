use gpui_agent_domain::{ChatRequest,
                        Job};
use serde::{Deserialize,
            Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobEvent {
    pub sequence: i64,
    pub event: StreamEvent,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobDetail {
    pub job: Job,
    pub request: ChatRequest,
}
use crate::StreamEvent;
