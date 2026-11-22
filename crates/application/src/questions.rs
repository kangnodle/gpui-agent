use crate::{Application,
            ApplicationError,
            ApplicationResult,
            ErrorKind,
            Jobs};
use gpui_agent_contracts::*;

impl Application {
    pub fn pending_questions(&self, operation_id: u64) -> ApplicationResult<Option<PendingQuestions>> {
        let operations = self
            .0
            .operations
            .lock()
            .map_err(|error| ApplicationError(ErrorKind::Failure, format!("Operation registry lock poisoned: {error}")))?;
        if !operations.contains_key(&operation_id) {
            return Ok(None);
        }
        let interactions = self
            .0
            .interactions
            .lock()
            .map_err(|error| ApplicationError(ErrorKind::Failure, format!("Interaction registry lock poisoned: {error}")))?;
        Ok(interactions.iter().find_map(|(&request_id, pending)| {
            if pending.operation_id != operation_id || pending.sender.is_closed() {
                return None;
            }
            if let Event::UserInput {
                questions,
            } = &pending.event
            {
                Some(PendingQuestions {
                    operation_id,
                    request_id,
                    questions: questions.clone(),
                })
            } else {
                None
            }
        }))
    }
}

impl Jobs {
    pub async fn answer_questions(&self, caller: Option<&McpScope>, request: QuestionAnswerRequest) -> ApplicationResult<()> {
        if caller.is_some() {
            return Err("User question replies require an operator credential".into());
        }
        if request.request_id == 0 {
            return Err("Request ID must be positive".into());
        }
        self.scoped_job(caller, request.id).await?;
        self.respond_interaction(request.id, request.request_id, request.response.into(), true)
    }
}
