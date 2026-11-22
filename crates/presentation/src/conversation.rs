//! Pure desktop state transitions and commands. No UI, networking or host I/O.
use gpui_agent_contracts::{AuthStatus,
                           ChatRequest,
                           ConversationSummary,
                           Event,
                           Model,
                           ProviderInfo,
                           SavedConversation,
                           SavedMessage as Message,
                           StreamEvent};

pub enum Update {
    McpProposals {
        scope: String,
        proposals: Vec<gpui_agent_contracts::McpProposal>,
    },
    SkillProposals {
        scope: String,
        proposals: Vec<gpui_agent_contracts::SkillProposal>,
    },
    GuidanceFiles {
        scope: String,
        catalog: gpui_agent_contracts::GuidanceFileCatalog,
    },
    GuidanceFile {
        scope: String,
        file: gpui_agent_contracts::GuidanceFile,
        saved: bool,
    },
    GuidanceImported {
        scope: String,
        text: String,
    },
    Guidance {
        scope: String,
        snapshot: gpui_agent_contracts::GuidanceSnapshot,
    },
    JobLoaded(gpui_agent_contracts::JobDetail, Option<SavedConversation>),
    Revised(gpui_agent_contracts::PreparedRevision),
    Versions(Vec<gpui_agent_contracts::ConversationVersion>),
    BackgroundJob(i64),
    Runtime(gpui_agent_contracts::RuntimeInfo),
    Git {
        overview: Box<gpui_agent_contracts::GitOverview>,
        restore: Option<String>,
    },
    Attached(Vec<gpui_agent_contracts::Attachment>),
    Projects(gpui_agent_contracts::ProjectCatalog),
    Changes(gpui_agent_contracts::GitChanges),
    Diff(gpui_agent_contracts::FileDiff),
    Providers(Vec<ProviderInfo>),
    Status(AuthStatus),
    Models(Vec<Model>),
    History {
        query: String,
        conversations: Vec<ConversationSummary>,
    },
    Loaded {
        conversation: SavedConversation,
        message_id: Option<i64>,
        query: String,
    },
    Referenced {
        conversation: SavedConversation,
        title: String,
    },
    Renamed {
        id: i64,
        title: String,
    },
    Deleted(i64),
    Event(StreamEvent),
    Error(String),
    InteractionSent {
        request_id: u64,
        result: Result<(), String>,
    },
    Finished,
}

pub enum Action {
    GuidanceFiles,
    SkillProposals,
    DecideMcpProposal {
        id: String,
        allow: bool,
    },
    DecideSkillProposal {
        id: String,
        allow: bool,
    },
    ReadGuidance(String),
    WriteGuidance(gpui_agent_contracts::GuidanceWriteRequest),
    ImportGuidance(std::path::PathBuf),
    ImportSkillFolder {
        path: std::path::PathBuf,
        id: String,
    },
    Guidance,
    SetSkillEnabled {
        id: String,
        enabled: bool,
    },
    SetProjectVerification {
        project: i64,
        plan: gpui_agent_contracts::ProjectVerification,
    },
    OpenJob(i64),
    Runtime {
        provider: gpui_agent_contracts::Provider,
        command: gpui_agent_contracts::RuntimeCommand,
    },
    Git {
        workspace: i64,
        command: gpui_agent_contracts::GitCommand,
    },
    State {
        id: i64,
        state: gpui_agent_contracts::ConversationState,
    },
    Context(gpui_agent_contracts::FileContextRequest),
    AttachFiles(Vec<std::path::PathBuf>),
    Projects,
    OpenProject(String),
    ForgetProject(i64),
    CreateWorktree {
        project_id: i64,
        branch: String,
        base: String,
    },
    RemoveWorktree(i64),
    Changes(i64),
    Diff {
        workspace_id: i64,
        path: String,
    },
    Refresh,
    Search(String),
    Login,
    Logout,
    Chat(ChatRequest),
    BackgroundChat(ChatRequest),
    Revise {
        extensions: Option<gpui_agent_contracts::ConversationExtensions>,
        skills: Vec<gpui_agent_contracts::SkillReference>,
        id: i64,
        expected_messages: usize,
        original: String,
        text: String,
        model: Option<String>,
        permission_mode: Option<gpui_agent_contracts::ClaudePermissionMode>,
        codex_execution_policy: Option<gpui_agent_contracts::CodexExecutionPolicy>,
        background: bool,
    },
    Load {
        id: i64,
        message_id: Option<i64>,
    },
    Reference {
        id: i64,
        title: String,
    },
    Rename {
        id: i64,
        title: String,
    },
    Delete(i64),
}

pub struct Draft {
    pub extensions: gpui_agent_contracts::ConversationExtensions,
    pub skills: Vec<gpui_agent_contracts::SkillReference>,
    pub conversation_id: Option<i64>,
    pub input: String,
    pub attachments: Vec<gpui_agent_contracts::Attachment>,
}

#[derive(Default)]
pub struct Conversation {
    pub guidance: crate::GuidanceState,
    pub account_id: Option<String>,
    pub editing: Option<String>,
    pub skills_before_edit: Option<Vec<gpui_agent_contracts::SkillReference>>,
    pub versions: Vec<gpui_agent_contracts::ConversationVersion>,
    pub drafts: Vec<Draft>,
    pub attachments: Vec<gpui_agent_contracts::Attachment>,
    pub pending: Option<crate::Pending>,
    pub timeline: Vec<gpui_agent_contracts::TimelineEntry>,
    pub id: Option<i64>,
    pub title: Option<String>,
    pub history: Vec<ConversationSummary>,
    pub show_archived: bool,
    pub search: String,
    pub query: String,
    pub view_query: String,
    pub scroll_to: Option<ScrollTarget>,
    pub status: Option<AuthStatus>,
    pub models: Vec<Model>,
    pub model: String,
    pub permission_mode: Option<gpui_agent_contracts::ClaudePermissionMode>,
    pub codex_execution_policy: Option<gpui_agent_contracts::CodexExecutionPolicy>,
    pub session_id: Option<String>,
    pub messages: Vec<Message>,
    pub input: String,
    pub authorization: Option<(String, Option<String>)>,
    pub error: Option<String>,
    pub assistant: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScrollTarget {
    Message(i64),
    Bottom,
}

impl Conversation {
    pub fn resume_job(&mut self, detail: &gpui_agent_contracts::JobDetail, saved: Option<SavedConversation>) {
        self.save_draft();
        self.new_chat();
        if let Some(saved) = saved {
            self.id = Some(saved.id);
            self.session_id = saved.session_id;
            self.messages = saved.messages;
            self.timeline = saved.timeline;
            if detail.job.operation_id.is_some()
                && let Some(question) = self.messages.iter().rposition(|message| message.user)
                && self.messages[question].text == detail.request.text
                && self.messages[question].attachments == detail.request.attachments
            {
                self.messages.truncate(question);
                self.timeline
                    .retain(|entry| self.messages.iter().any(|message| message.id == Some(entry.message_id)));
            }
        }
        self.id = detail.job.conversation_id.or(self.id);
        self.title = Some(detail.job.preview.clone());
        self.model = detail.request.model.clone().unwrap_or_default();
        self.permission_mode = detail.request.claude_permission_mode;
        self.codex_execution_policy = detail.request.codex_execution_policy;
        self.begin_turn(&detail.request);
    }

    pub fn last_completed_question(&self) -> Option<usize> {
        let last = self.messages.last()?;
        if self.id.is_none() || last.user || self.messages.iter().any(|message| message.incomplete) || self.pending.is_some() {
            return None;
        }
        self.messages.iter().rposition(|message| message.user)
    }

    pub fn revision_changed(&self) -> bool {
        let (Some(text), Some(index)) = (&self.editing, self.last_completed_question()) else {
            return false;
        };
        let original = &self.messages[index];
        let extensions = original.guidance.as_ref().and_then(|receipt| receipt.extensions.clone()).unwrap_or_default();
        (text != &original.text || self.guidance.selected != original.skills || self.guidance.extensions != extensions)
            && (!text.trim().is_empty() || !original.attachments.is_empty())
    }

    pub fn begin_turn(&mut self, request: &ChatRequest) {
        if let Some(selection) = &request.extensions {
            self.guidance.extensions = selection.clone();
            self.guidance.selected = request.skills.clone();
        }
        self.messages.push(Message {
            skills: request.skills.clone(),
            guidance: None,
            attachments: request.attachments.clone(),
            id: None,
            user: true,
            text: request.text.clone(),
            incomplete: false,
        });
        self.assistant = Some(self.messages.len());
        self.messages.push(Message {
            skills: vec![],
            guidance: None,
            attachments: vec![],
            id: Some(-(self.messages.len() as i64) - 1),
            user: false,
            text: String::new(),
            incomplete: true,
        });
        self.scroll_to = Some(ScrollTarget::Bottom);
    }

    pub fn save_draft(&mut self) {
        let selection_changed = self.messages.iter().rev().find(|message| message.user).is_some_and(|message| {
            message
                .guidance
                .as_ref()
                .and_then(|receipt| receipt.extensions.as_ref())
                .is_some_and(|saved| saved != &self.guidance.extensions || message.skills != self.guidance.selected)
        });
        if self.input.is_empty()
            && self.attachments.is_empty()
            && self.guidance.selected.is_empty()
            && self.guidance.extensions == Default::default()
            && !selection_changed
        {
            return;
        }
        if self.id.is_some() {
            self.drafts.retain(|draft| draft.conversation_id != self.id);
        }
        self.drafts.push(Draft {
            extensions: std::mem::take(&mut self.guidance.extensions),
            skills: std::mem::take(&mut self.guidance.selected),
            conversation_id: self.id,
            input: std::mem::take(&mut self.input),
            attachments: std::mem::take(&mut self.attachments),
        });
    }

    pub fn new_chat(&mut self) {
        self.skills_before_edit = None;
        self.guidance.selected.clear();
        self.guidance.extensions = Default::default();
        self.editing = None;
        self.versions.clear();
        self.title = None;
        self.id = None;
        self.session_id = None;
        self.messages.clear();
        self.timeline.clear();
        self.pending = None;
        self.assistant = None;
        self.error = None;
        self.input.clear();
        self.attachments.clear();
        self.view_query.clear();
        self.scroll_to = None;
    }

    pub fn event(&mut self, event: StreamEvent) {
        if let StreamEvent::Agent(
            event @ (Event::Tool {
                ..
            }
            | Event::Verification {
                ..
            }
            | Event::Permission {
                ..
            }
            | Event::UserInput {
                ..
            }
            | Event::Elicitation {
                ..
            }
            | Event::InteractionResolved {
                ..
            }
            | Event::Usage {
                ..
            }
            | Event::RuntimeStatus {
                ..
            }
            | Event::InputDelivery {
                ..
            }
            | Event::TurnEnd {
                ..
            }),
        ) = &event
        {
            let message_id = if matches!(event, Event::Usage { .. }) {
                self.assistant.map(|index| self.messages[index].id.unwrap_or(-(index as i64) - 1)).unwrap_or(-1)
            } else {
                self.assistant.and_then(|index| self.messages[index].id).unwrap_or(-1)
            };
            self.timeline.push(gpui_agent_contracts::TimelineEntry {
                message_id,
                event: event.clone(),
            });
        }
        if event.is_terminal() {
            self.pending = None;
            let message_id = self.assistant.and_then(|index| self.messages[index].id).unwrap_or(-1);
            self.timeline.push(gpui_agent_contracts::TimelineEntry {
                message_id,
                event: Event::TurnEnd {
                    completed: matches!(event, StreamEvent::Completed { .. }),
                },
            });
        }
        match event {
            | StreamEvent::Conversation {
                conversation_id,
            } => self.id = Some(conversation_id),
            | StreamEvent::Agent(Event::Session {
                id,
            }) => self.session_id = Some(id),
            | StreamEvent::Agent(Event::Text {
                delta,
            }) =>
                if let Some(index) = self.assistant {
                    self.messages[index].text.push_str(&delta);
                },
            | StreamEvent::Agent(Event::Authorization {
                url,
                code,
            }) => self.authorization = Some((url, code)),
            | StreamEvent::Authenticated {
                status,
            } => {
                self.status = Some(status);
                self.authorization = None;
                self.new_chat();
            },
            | StreamEvent::Completed {
                session_id,
                text,
            } => {
                self.session_id = Some(session_id);
                if let Some(index) = self.assistant {
                    self.messages[index].text = text;
                    self.messages[index].incomplete = false;
                }
            },
            | StreamEvent::Error {
                message,
            } => {
                self.error = Some(message);
                self.authorization = None;
            },
            | StreamEvent::Cancelled => {
                self.error = Some("작업을 중지했습니다.".into());
                self.authorization = None;
            },
            | StreamEvent::Interaction {
                operation_id,
                request_id,
                event,
            } => {
                let mut pending = crate::Pending::new(operation_id, request_id, event);
                pending.mode = self.permission_mode;
                self.pending = Some(pending);
            },
            | StreamEvent::Agent(Event::InteractionResolved {
                request_id,
                response,
            }) => {
                if let gpui_agent_contracts::InteractionResponse::ClaudeMode {
                    mode,
                } = response
                {
                    self.permission_mode = (mode != gpui_agent_contracts::ClaudePermissionMode::Default).then_some(mode);
                }
                if self.pending.as_ref().is_some_and(|pending| pending.request_id == request_id) {
                    self.pending = None;
                }
            },
            | StreamEvent::Agent(_) => {},
            | StreamEvent::Started {
                ..
            } => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resumed_job_keeps_prior_turns_and_replays_its_current_turn_once() {
        let detail: gpui_agent_contracts::JobDetail = serde_json::from_value(serde_json::json!({
            "job":{"id":5,"scope":{"provider":"claude","account_id":"default","workspace_id":42,"workspace_path":"/first"},
                "state":"running","conversation_id":7,"operation_id":42,"pending_request_id":null,"last_sequence":3,
                "preview":"Current question","cancel_requested":false,"error":null,"created_at":1},
            "request":{"codex_execution_policy":{"approval_policy":"on-request","sandbox":"workspace-write"},"text":"Current question","conversation_id":7,"session_id":"previous-session","model":null}
        }))
        .unwrap();
        let saved = serde_json::from_value(serde_json::json!({"id":7,"session_id":"previous-session","model":null,"messages":[
            {"id":1,"user":true,"text":"Earlier question","incomplete":false},
            {"id":2,"user":false,"text":"Earlier answer","incomplete":false},
            {"id":3,"user":true,"text":"Current question","incomplete":false},
            {"id":4,"user":false,"text":"Partial text","incomplete":true}
        ]}))
        .unwrap();
        let mut conversation = Conversation {
            input: "Unsent draft".into(),
            ..Default::default()
        };
        conversation.resume_job(&detail, Some(saved));
        assert_eq!(conversation.codex_execution_policy, detail.request.codex_execution_policy);
        assert!(conversation.codex_execution_policy.is_some());
        conversation.event(StreamEvent::Agent(Event::Text {
            delta: "Replayed text".into(),
        }));
        assert_eq!(conversation.messages.len(), 4);
        assert_eq!(conversation.messages[1].text, "Earlier answer");
        assert_eq!(conversation.messages[2].text, "Current question");
        assert_eq!(conversation.messages[3].text, "Replayed text");
        assert_eq!(conversation.session_id.as_deref(), Some("previous-session"));
        assert_eq!(conversation.drafts[0].input, "Unsent draft");
    }

    #[test]
    fn only_the_last_completed_question_can_be_edited() {
        let mut conversation = Conversation::default();
        let request = ChatRequest {
            extensions: None,
            skills: vec![],
            computer_use: false,
            text: "Question".into(),
            attachments: vec![],
            model: None,
            session_id: None,
            conversation_id: None,
            claude_permission_mode: None,
            codex_execution_policy: None,
        };
        assert_eq!(conversation.last_completed_question(), None);
        conversation.begin_turn(&request);
        conversation.id = Some(7);
        assert_eq!(conversation.last_completed_question(), None);
        conversation.event(StreamEvent::Completed {
            session_id: "session".into(),
            text: "Answer".into(),
        });
        assert_eq!(conversation.last_completed_question(), Some(0));
        conversation.begin_turn(&request);
        assert_eq!(conversation.last_completed_question(), None);
        conversation.event(StreamEvent::Completed {
            session_id: "session".into(),
            text: "Another answer".into(),
        });
        assert_eq!(conversation.last_completed_question(), Some(2));
        conversation.new_chat();
        assert_eq!(conversation.last_completed_question(), None);
    }
    #[test]
    fn saved_drafts_keep_attachments_and_do_not_overwrite_other_unsent_chats() {
        let mut conversation = Conversation::default();
        conversation.save_draft();
        assert!(conversation.drafts.is_empty());
        for input in ["First draft", "Second draft"] {
            conversation.input = input.into();
            conversation.save_draft();
            conversation.new_chat();
        }
        assert_eq!(conversation.drafts[0].input, "First draft");
        assert_eq!(conversation.drafts[1].input, "Second draft");
        conversation.id = Some(7);
        conversation.attachments.push(gpui_agent_contracts::Attachment::Text {
            name: "context.txt".into(),
            text: "Keep this".into(),
            source: None,
        });
        conversation.save_draft();
        assert_eq!(conversation.drafts[2].attachments.len(), 1);
        conversation.input = "Updated draft".into();
        conversation.save_draft();
        assert_eq!(conversation.drafts.len(), 3);
        assert_eq!(conversation.drafts[2].input, "Updated draft");
        assert!(conversation.input.is_empty());
    }

    #[test]
    fn chosen_claude_mode_remains_selected_for_the_next_turn() {
        let mut conversation = Conversation::default();
        for mode in gpui_agent_contracts::ClaudePermissionMode::ALL {
            conversation.event(StreamEvent::Agent(Event::InteractionResolved {
                request_id: 1,
                response: gpui_agent_contracts::InteractionResponse::ClaudeMode {
                    mode,
                },
            }));
            assert_eq!(conversation.permission_mode.unwrap_or_default(), mode);
        }
    }
    #[test]
    fn separate_live_turns_do_not_share_a_usage_snapshot_key() {
        let usage = gpui_agent_contracts::Usage {
            scope: gpui_agent_contracts::UsageScope::Turn,
            input_tokens: Some(10),
            output_tokens: Some(2),
            total_tokens: Some(12),
            cached_input_tokens: None,
            cache_write_tokens: None,
            context_window: None,
            cost_usd: None,
        };
        let mut state = Conversation::default();
        for _ in 0 .. 2 {
            state.messages.push(Message {
                skills: vec![],
                guidance: None,
                id: None,
                user: false,
                text: String::new(),
                incomplete: true,
                attachments: vec![],
            });
            state.assistant = Some(state.messages.len() - 1);
            state.event(StreamEvent::Agent(Event::Usage {
                usage: usage.clone(),
            }));
            state.event(StreamEvent::Agent(Event::Usage {
                usage: usage.clone(),
            }));
        }
        let report = gpui_agent_contracts::UsageReport::recorded(&state.timeline, 2);
        assert_eq!(report.reported_turns, 2);
        assert_eq!(report.usage.unwrap().total_tokens, Some(24));
    }

    #[test]
    fn streamed_completion_does_not_duplicate_text_and_failed_turns_stay_partial() {
        let mut conversations: [Conversation; gpui_agent_contracts::PROVIDERS.len()] = std::array::from_fn(|_| Conversation::default());
        let conversation = &mut conversations[0];
        conversation.messages.push(Message {
            skills: vec![],
            guidance: None,
            attachments: Vec::new(),
            id: None,
            user: false,
            text: String::new(),
            incomplete: true,
        });
        conversation.assistant = Some(0);
        conversation.event(StreamEvent::Agent(Event::Session {
            id: "claude-session".into(),
        }));
        conversation.event(StreamEvent::Agent(Event::Text {
            delta: "안녕".into(),
        }));
        conversation.event(StreamEvent::Completed {
            session_id: "claude-session".into(),
            text: "안녕하세요".into(),
        });
        assert_eq!(conversation.messages[0].text, "안녕하세요");
        assert!(!conversation.messages[0].incomplete);
        assert!(conversations[1].session_id.is_none());
        conversations[2].messages.push(Message {
            skills: vec![],
            guidance: None,
            attachments: Vec::new(),
            id: None,
            user: false,
            text: "부분 응답".into(),
            incomplete: true,
        });
        conversations[2].assistant = Some(0);
        conversations[2].event(StreamEvent::Error {
            message: "disconnected".into(),
        });
        assert!(conversations[2].messages[0].incomplete);
        assert_eq!(conversations[2].messages[0].text, "부분 응답");
    }
}
