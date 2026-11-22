use crate::{InteractionResponse,
            Question};
use serde::{Deserialize,
            Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
pub struct PendingQuestions {
    pub operation_id: u64,
    pub request_id: u64,
    pub questions: Vec<Question>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum QuestionResponse {
    Answers { answers: BTreeMap<String, Vec<String>> },
    Cancelled,
}
impl From<QuestionResponse> for InteractionResponse {
    fn from(response: QuestionResponse) -> Self {
        match response {
            | QuestionResponse::Answers {
                answers,
            } => Self::Answers {
                answers,
            },
            | QuestionResponse::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionAnswerRequest {
    pub id: i64,
    pub request_id: u64,
    pub response: QuestionResponse,
}
