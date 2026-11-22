use crate::{ConversationTarget,
            PullRequestTarget};
use serde::{Deserialize,
            Serialize};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PullRequestConversation {
    #[serde(flatten)]
    pub scope: ConversationTarget,
    pub id: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PullRequestInput {
    pub url: Option<String>,
    pub repository: Option<String>,
    pub number: Option<u32>,
}
impl PullRequestInput {
    pub fn target(&self) -> Result<PullRequestTarget, &'static str> { PullRequestTarget::parse(self.url.as_deref(), self.repository.as_deref(), self.number) }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WritePullRequestGuides {
    #[serde(flatten)]
    pub scope: ConversationTarget,
    pub pull_requests: Vec<PullRequestInput>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadPullRequestGuide {
    #[serde(flatten)]
    pub scope: ConversationTarget,
    pub url: Option<String>,
    pub repository: Option<String>,
    pub number: Option<u32>,
}
#[derive(Debug, Serialize)]
pub struct PullRequestGuideOutcome {
    pub target: Option<PullRequestTarget>,
    pub outcome: &'static str,
    pub detail: Option<String>,
    pub run: Option<crate::PullRequestGuide>,
}
#[derive(Debug, Serialize)]
pub struct PullRequestGuideRead {
    pub target: PullRequestTarget,
    pub run: Option<crate::PullRequestGuide>,
    pub current_head_sha: Option<String>,
    pub current_base_sha: Option<String>,
    pub stale: Option<bool>,
    pub read_error: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PullRequestCommand {
    #[serde(flatten)]
    pub scope: ConversationTarget,
    pub id: i64,
    pub url: Option<String>,
    pub repository: Option<String>,
    pub number: Option<u32>,
}
impl PullRequestCommand {
    pub fn target(&self) -> Result<PullRequestTarget, &'static str> { PullRequestTarget::parse(self.url.as_deref(), self.repository.as_deref(), self.number) }
}
