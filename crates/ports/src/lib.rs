#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]

mod clock;
mod control;
mod conversation;
mod diagnostics;
mod files;
mod guidance;
mod mcp;
mod plugins;
mod runtime;
mod verifier;

pub use clock::Clock;
pub use control::{ComputerSession,
                  ControlValidity,
                  HostControl};
pub use conversation::{ConversationRepository,
                       EditOutcome,
                       ForkOutcome,
                       Turn};
pub use diagnostics::DiagnosticSink;
pub use files::FileContextReader;
pub use guidance::GuidanceSource;
pub use mcp::{McpConnection,
              McpDiagnosticSink,
              McpSessionAuthorizer,
              McpSessionIssuer,
              McpSessionLease};
pub use plugins::PluginSource;
pub use runtime::{AgentRuntime,
                  RuntimeFuture,
                  RuntimeInput,
                  TurnInput};
pub use verifier::ProjectVerifier;
