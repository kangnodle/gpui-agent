#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
//! Deterministic client state and commands. Rendering and I/O belong to the
//! host.
mod conversation;
mod display;
pub use display::{is_active,
                  plain,
                  state_label};
mod guidance;
mod history;
pub use guidance::{ConversationExtensionEntry,
                   ExtensionKind,
                   GuidanceEditor,
                   GuidanceState,
                   InstructionStatus,
                   guidance_display_path,
                   guidance_scope,
                   instruction_status,
                   mcp_proposal_facts,
                   skill_groups,
                   skill_in_scope,
                   skill_proposal_files};
mod interaction;
mod language;
mod verification;
pub use language::Language;
pub use verification::{VerificationDraft,
                       format_command_line,
                       split_command_line};
mod jobs;
mod panels;
pub use conversation::{Action,
                       Conversation,
                       Draft,
                       ScrollTarget,
                       Update};
pub use history::{HistoryEntry,
                  HistoryGroup,
                  history_entries};
pub use interaction::{Pending,
                      computer_observation,
                      is_computer_observation,
                      is_work_event,
                      tools,
                      verification_reports};
pub use jobs::JobActivity;
pub use panels::RightPanels;
