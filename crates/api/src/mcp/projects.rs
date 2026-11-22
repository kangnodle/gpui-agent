use super::*;

pub(super) fn tools() -> Vec<ToolDefinition> {
    let workspace = json!({"type":"integer","minimum":1,"description":"Defaults to the session workspace, or the catalog default for an operator."});
    let id = json!({"type":"integer","minimum":1});
    vec![
        ToolDefinition {
            name: "workspace_propose_commit",
            description: "Submit a pending commit proposal after editing files, then finish your work. This only saves intent: it does not stage files or commit. The operator reviews the diff, selects paths, edits the message and explicitly approves outside the agent. Never claim that a proposed commit has completed. Scoped sessions can propose only in their own workspace without general write authority.",
            schema: json!({"type":"object","properties":{"workspace":workspace,"message":{"type":"string","minLength":1,"maxLength":8192},"paths":{"type":"array","minItems":1,"maxItems":4096,"items":{"type":"string","minLength":1,"maxLength":32768}}},"required":["message","paths"],"additionalProperties":false}),
            read_only: false,
        },
        ToolDefinition {
            name: "project_list",
            description: "List registered projects and worktrees, including branch, managed, locked and missing status. Agent sessions see only their own workspace. Operator credentials see the full catalog. Workspace IDs may be used with conversation and job tools; listing does not change session scope.",
            schema: json!({"type":"object","properties":{},"additionalProperties":false}),
            read_only: true,
        },
        ToolDefinition {
            name: "project_register",
            description: "Operator only: open an existing absolute directory and return the catalog with registered_workspace_id. Non-Git folders are supported. Reopening a hidden project restores its IDs and history; repeating registration is safe. Does not switch an agent's workspace or grant access to it.",
            schema: json!({"type":"object","properties":{"path":{"type":"string","minLength":1,"maxLength":32768}},"required":["path"],"additionalProperties":false}),
            read_only: false,
        },
        ToolDefinition {
            name: "project_forget",
            description: "Operator only: hide a project and all its worktrees from the catalog. Preserves folders, branches and history. Pending/running jobs or active operations prevent removal. Reopen with project_register to restore. Use worktree_remove to delete a managed worktree directory.",
            schema: json!({"type":"object","properties":{"id":id},"required":["id"],"additionalProperties":false}),
            read_only: false,
        },
        ToolDefinition {
            name: "worktree_create",
            description: "Operator only: create a new branch and worktree in the app's managed directory for a registered Git project. Omitted/empty base uses environment_read.preferences.default_worktree_base (initially HEAD); an explicit base wins and must identify an existing commit. Existing branches are never overwritten. Returns the catalog; locate the new workspace by branch. Not idempotent: after uncertain delivery inspect project_list before retrying. Does not change session scope.",
            schema: json!({"type":"object","properties":{"project":id,"branch":{"type":"string","minLength":1,"maxLength":256},"base":{"type":"string","maxLength":512}},"required":["project","branch"],"additionalProperties":false}),
            read_only: false,
        },
        ToolDefinition {
            name: "worktree_remove",
            description: "Operator only: delete a clean, idle worktree created by this app. Main, external, locked or unmanaged directories are rejected; changed, untracked or ignored files, pending/running jobs and staged uploads prevent deletion. Delete unused uploads first. No force option. Branches and conversation history are preserved. Missing IDs return an error; inspect project_list after uncertain delivery.",
            schema: json!({"type":"object","properties":{"id":id},"required":["id"],"additionalProperties":false}),
            read_only: false,
        },
        ToolDefinition {
            name: "workspace_changes",
            description: "Read the workspace branch and changed files with index/worktree status and original paths for renames. Agent sessions cannot read another workspace, including sibling worktrees. Does not stage or modify files.",
            schema: json!({"type":"object","properties":{"workspace":workspace},"additionalProperties":false}),
            read_only: true,
        },
        ToolDefinition {
            name: "workspace_diff",
            description: "Read staged and unstaged diff for an exact path returned by workspace_changes, including untracked files. Each output is bounded to 2 MiB and reports truncation. Hooks, external diff and textconv are disabled. Paths are literal; no shell or pathspec expansion.",
            schema: json!({"type":"object","properties":{"workspace":workspace,"path":{"type":"string","minLength":1,"maxLength":32768}},"required":["path"],"additionalProperties":false}),
            read_only: true,
        },
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceRead {
    workspace: Option<i64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiffRead {
    workspace: Option<i64>,
    path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommitPropose {
    workspace: Option<i64>,
    message: String,
    paths: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorktreeCreate {
    project: i64,
    branch: String,
    base: Option<String>,
}

pub(super) fn operator_only(name: &str) -> bool { matches!(name, "project_register" | "project_forget" | "worktree_create" | "worktree_remove") }

pub(super) async fn execute(api: &Api, name: &str, args: Value, access: &Access) -> Result<Executed, ApplicationError> {
    let service = api
        .workspaces
        .as_ref()
        .ok_or(ApplicationError(ErrorKind::Unavailable, "Project management is unavailable".into()))?;
    match name {
        | "workspace_propose_commit" => {
            let args: CommitPropose = decode(args)?;
            encode(
                service
                    .propose_commit_for_session(args.workspace, access.scope.as_ref(), args.message, args.paths)
                    .await?,
                None,
            )
        },
        | "project_list" => {
            let _: Empty = decode(args)?;
            encode(service.catalog_for_session(access.scope.as_ref()).await?, None)
        },
        | "project_register" => {
            let args: RegisterProject = decode(args)?;
            encode(service.register(args.path).await?, None)
        },
        | "project_forget" => {
            let args: JobId = decode(args)?;
            encode(service.forget_project_if_idle(args.id, api.jobs.clone()).await?, None)
        },
        | "worktree_create" => {
            let args: WorktreeCreate = decode(args)?;
            encode(service.create(args.project, args.branch, args.base.unwrap_or_default()).await?, None)
        },
        | "worktree_remove" => {
            let args: JobId = decode(args)?;
            encode(service.remove_if_idle(args.id, api.jobs.clone()).await?, None)
        },
        | "workspace_changes" => {
            let args: WorkspaceRead = decode(args)?;
            encode(service.changes_for_session(args.workspace, access.scope.as_ref()).await?, None)
        },
        | "workspace_diff" => {
            let args: DiffRead = decode(args)?;
            encode(service.diff_for_session(args.workspace, access.scope.as_ref(), args.path).await?, None)
        },
        | _ => Err("Unknown project tool".into()),
    }
}
