use super::*;

pub(super) fn tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "host_mcp_propose",
            description: "Propose adding one MCP server to this session's workspace. Nothing is installed or started until the operator approves; a proposal is not a granted capability. server uses the host McpServer format: stdio {name, command (absolute path to an executable that exists on the host), args?} or http {name, url (https, or loopback http), bearer_token_env_var?, env_http_headers?}; the env fields only name environment variables, never carry secret values. OAuth servers are not accepted. rationale says why the server is needed. On approval it is installed as the workspace plugin repo:mcp-<name> and enabled. Returns a pending proposal id immediately. You may end your turn instead of polling: the server starts with the next turn, and when the operator approves, the host queues that continuation turn in this conversation automatically. Rejection queues nothing; host_mcp_proposal_status reports it. At most 8 undecided proposals per session scope; they expire after 30 minutes.",
            schema: json!({"type":"object","properties":{"server":{"oneOf":[{"type":"object","properties":{"name":{"type":"string","minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9_-]+$"},"command":{"type":"string","minLength":1,"maxLength":4096},"args":{"type":"array","maxItems":64,"items":{"type":"string","maxLength":4096}}},"required":["name","command"],"additionalProperties":false},{"type":"object","properties":{"name":{"type":"string","minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9_-]+$"},"url":{"type":"string","minLength":1,"maxLength":4096},"bearer_token_env_var":{"type":"string","maxLength":128},"env_http_headers":{"type":"object","maxProperties":16,"additionalProperties":{"type":"string","maxLength":128}}},"required":["name","url"],"additionalProperties":false}]},"rationale":{"type":"string","minLength":1,"maxLength":1000}},"required":["server","rationale"],"additionalProperties":false}),
            read_only: false,
        },
        ToolDefinition {
            name: "host_mcp_proposal_status",
            description: "Read the status of an MCP server proposal made in this session scope: pending, applying, applied, rejected, failed (message explains why) or expired. Reads do not decide it. Only the operator can approve or reject. An applied server is available from the next conversation turn.",
            schema: json!({"type":"object","properties":{"id":{"type":"string","minLength":1,"maxLength":32}},"required":["id"],"additionalProperties":false}),
            read_only: true,
        },
    ]
}

/// This process is the host: executables and credential variables are checked
/// where the provider turns will actually run. Variable values are tested for
/// usability and dropped; they are never stored, logged or returned.
pub(crate) struct LocalHost;
impl gpui_agent_application::McpHost for LocalHost {
    fn executable_exists(&self, command: &str) -> bool { std::path::Path::new(command).is_file() }

    fn environment_ready(&self, variable: &str) -> bool { std::env::var(variable).is_ok_and(|value| gpui_agent_contracts::valid_mcp_header_value(&value)) }
}

pub(super) async fn execute(api: &Api, name: &str, args: Value, access: &Access) -> Result<Executed, ApplicationError> {
    let scope = access
        .scope
        .as_ref()
        .ok_or(ApplicationError(ErrorKind::Invalid, "MCP proposals require an agent session".into()))?;
    match name {
        | "host_mcp_propose" => encode(
            api.mcp_proposals
                .propose(scope, super::guidance::proposal_origin(api, access, scope).await, decode(args)?, &LocalHost)?,
            None,
        ),
        | _ => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Id {
                id: String,
            }
            let Id {
                id,
            } = decode(args)?;
            encode(api.mcp_proposals.status(Some(scope), &id)?, None)
        },
    }
}
