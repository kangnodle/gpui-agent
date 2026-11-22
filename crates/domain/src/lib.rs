#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod attachment;
pub mod bounded_contexts;
mod checkpoint;
mod codex_policy;
mod computer_lease;
mod context_transfer;
pub mod conversation;
mod conversation_view;
mod elicitation;
mod environment;
mod extensions;
mod guidance;
mod interaction;
mod job;
mod maintenance;
mod mcp;
mod mcp_oauth;
mod plugins;
mod proposal;
pub mod provider;
mod pull_request;
mod queue;
mod revision;
pub mod runtime;
pub mod search;
mod snapshot;
pub mod state_machine;
mod usage;
mod validation;
mod verification;
pub mod workspace;

pub use attachment::{Attachment,
                     FileContextRequest,
                     UploadedAttachment,
                     WorkspaceFile,
                     WorkspaceFiles,
                     WorkspaceFilesRequest,
                     validate_attachments};
pub use checkpoint::*;
pub use codex_policy::*;
pub use computer_lease::{ComputerInputTarget,
                         ComputerLeaseInput,
                         ComputerLeaseState,
                         ScreenObservation,
                         transition_computer_lease};
pub use context_transfer::{ContextTransfer,
                           ContextTransferCommand,
                           ContextTransferError,
                           ContextTransferKind,
                           ContextTransferPlan,
                           ContextTransferState};
pub use conversation::{ConversationState,
                       ConversationSummary,
                       ConversationVersion,
                       MatchedMessage,
                       SavedConversation,
                       SavedMessage,
                       TimelineEntry};
pub use conversation_view::{ConversationArchiveFilter,
                            ConversationCursor,
                            ConversationListPage,
                            ConversationMessage,
                            ConversationPage};
pub use environment::*;
pub use extensions::*;
pub use guidance::*;
pub use interaction::validate_response;
pub use job::{AccountProfile,
              ChatRequest,
              Job,
              JobMachine,
              JobScope,
              JobState,
              JobTransitionInput,
              JobTransitionOutcome,
              transition_job,
              validate_account_id,
              validate_command_id};
pub use maintenance::{RuntimeCommand,
                      RuntimeInfo,
                      RuntimeRelease,
                      valid_runtime_version};
pub use mcp::{McpDiagnostic,
              McpGrant,
              McpScope,
              McpServer,
              TaskSession,
              merge_mcp,
              valid_mcp_header_value,
              validate_mcp};
pub use mcp_oauth::*;
pub use plugins::*;
pub use proposal::{ProposalAuditEvent,
                   ProposalLedgerEntry,
                   ProposalLedgerError,
                   ProposalLedgerLimits,
                   ProposalLedgerState,
                   ProposalLifecycle,
                   ProposalStatus,
                   ProposalTransitionError,
                   ProposalTransitionInput,
                   transition_proposal};
pub use provider::{PROVIDERS,
                   Provider,
                   display_name,
                   provider_index,
                   provider_name};
pub use pull_request::*;
pub use queue::{QueueError,
                validate_queue_capacity};
pub use revision::{ReviseRequest,
                   RevisionError,
                   RevisionPlan};
pub use runtime::{AuthStatus,
                  ClaudePermissionMode,
                  Completion,
                  ElicitationAction,
                  EmitFuture,
                  Event,
                  EventHandler,
                  Handler,
                  InteractionResponse,
                  Model,
                  Prompt,
                  Question,
                  ResponseFuture,
                  RuntimeStatus,
                  ToolActivity,
                  ToolCall};
pub use search::match_ranges;
pub use snapshot::{SnapshotError,
                   conversation_snapshot};
pub use state_machine::{Transition,
                        fold_transitions};
pub use usage::{Usage,
                UsageReport,
                UsageScope};
pub use validation::{conversation_title,
                     normalize_title,
                     validate_chat,
                     validate_login,
                     validate_message,
                     validate_search};
pub use verification::*;
pub use workspace::{ChangedFile,
                    FileDiff,
                    GitChanges,
                    Project,
                    ProjectCatalog,
                    Workspace};
