use super::*;

pub(super) fn tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "environment_read",
            description: "Read this host's stable environment ID, server version, OS/architecture and selected preferences. Excludes process environment variables, credentials, home paths and executable paths. Provider/model availability is exposed by orchestrator_capabilities. Does not change the caller's scope.",
            schema: json!({"type":"object","properties":{},"additionalProperties":false}),
            read_only: true,
        },
        ToolDefinition {
            name: "environment_preferences_update",
            description: "Operator only: atomically patch this host's display label and default_worktree_base. Omitted/null fields are preserved; empty patches and blank values are rejected. Persists across restarts. The base is a Git revision expression (default HEAD), checked for existence in each project when creating a worktree. Changes affect future worktrees with omitted/empty base; explicit bases win. No credential, executable, process environment, permission or network settings are writable here. Scoped agent sessions cannot change host-wide preferences.",
            schema: json!({"type":"object","properties":{"label":{"type":"string","minLength":1,"maxLength":100},"default_worktree_base":{"type":"string","minLength":1,"maxLength":512}},"minProperties":1,"additionalProperties":false}),
            read_only: false,
        },
    ]
}

pub(super) fn operator_only(name: &str) -> bool { name == "environment_preferences_update" }

pub(super) async fn execute(api: &Api, name: &str, args: Value, access: &Access) -> Result<Executed, ApplicationError> {
    let service = api
        .workspaces
        .as_ref()
        .ok_or(ApplicationError(ErrorKind::Unavailable, "Environment settings unavailable".into()))?;
    match name {
        | "environment_read" => {
            let _: Empty = decode(args)?;
            encode(service.environment_read().await?, None)
        },
        | "environment_preferences_update" => encode(service.environment_preferences_update(access.scope.as_ref(), decode(args)?).await?, None),
        | _ => Err("Unknown environment tool".into()),
    }
}
