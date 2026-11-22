use super::*;

pub(super) fn tools() -> Vec<ToolDefinition> {
    let conversation = json!({"id":{"type":"integer","minimum":1,"description":"Saved conversation ID in the selected scope."}});
    let mut target = conversation.clone();
    target["url"] = json!({"type":"string","maxLength":512,"description":"Canonical https://github.com/owner/repo/pull/number URL; alternatively supply repository and number."});
    target["repository"] = json!({"type":"string","maxLength":201});
    target["number"] = json!({"type":"integer","minimum":1,"maximum":2147483647});
    let mut guide_target = target.clone();
    guide_target.as_object_mut().expect("object").remove("id");
    let batch =
        json!({"pull_requests":{"type":"array","minItems":1,"maxItems":20,"items":{"type":"object","properties":guide_target,"additionalProperties":false}}});
    vec![
        ToolDefinition {
            name: "write_pull_request_guides",
            description: "Queue reviewer guides for 1–20 GitHub PRs matching a workspace remote. Each target accepts url or repository and number. Reads a bounded diff with stable head/base, then starts a separate normal provider job using that snapshot. Active or successful runs for the same scope/PR/head/base are reused. Inspect read_pull_request_guide or job tools for completion/approvals; this call returns acceptance. Does not post reviews to GitHub. Same-provider/account jobs wait behind the current turn.",
            schema: conversations::schema(batch, &["pull_requests"]),
            read_only: false,
        },
        ToolDefinition {
            name: "read_pull_request_guide",
            description: "Read the latest local guide job/result and compare head/base with GitHub. Never starts generation. stale=null with read_error means current commits could not be verified. Guides are scoped to provider/account/workspace and use remote diff snapshots; they do not prove tests passed or that the local checkout matches the PR.",
            schema: conversations::schema(guide_target, &[]),
            read_only: true,
        },
        ToolDefinition {
            name: "link_pull_request",
            description: "Link a GitHub PR to a saved conversation. Requires either url or repository and number. Idempotent; already_linked reports the previous state. Saves local intent without writing to GitHub or starting an agent.",
            schema: conversations::schema(target.clone(), &["id"]),
            read_only: false,
        },
        ToolDefinition {
            name: "watch_pull_request",
            description: "Link and watch an open GitHub PR matching a workspace remote. The current snapshot is the baseline. Polls every two minutes while the host runs; new check results, comments/reviews and conflicts queue a normal conversation turn. No automatic merge or permission approval. Repeating an active watch preserves its baseline.",
            schema: conversations::schema(target.clone(), &["id"]),
            read_only: false,
        },
        ToolDefinition {
            name: "unwatch_pull_request",
            description: "Stop a local PR watch, keeping its link. Idempotent; does not cancel already queued notifications.",
            schema: conversations::schema(target.clone(), &["id"]),
            read_only: false,
        },
        ToolDefinition {
            name: "unlink_pull_request",
            description: "Remove a conversation's local PR link and its watch. Idempotent; was_linked reports the previous state. Does not close the PR or cancel already queued work.",
            schema: conversations::schema(target, &["id"]),
            read_only: false,
        },
        ToolDefinition {
            name: "list_thread_pull_requests",
            description: "Read a saved conversation's local PR links in this provider/account/workspace. Requires an explicit conversation id. Listing does not contact GitHub.",
            schema: conversations::schema(conversation, &["id"]),
            read_only: true,
        },
    ]
}

pub(super) async fn execute(jobs: &gpui_agent_application::Jobs, name: &str, args: Value, access: &Access) -> Result<Executed, ApplicationError> {
    match name {
        | "write_pull_request_guides" => encode(
            json!({"guides":jobs.write_pull_request_guides(access.scope.as_ref(),decode(args)?).await?}),
            None,
        ),
        | "read_pull_request_guide" => encode(jobs.read_pull_request_guide(access.scope.as_ref(), decode(args)?).await?, None),
        | "watch_pull_request" | "unwatch_pull_request" => {
            let watching = name == "watch_pull_request";
            let was_watching = jobs.set_pull_request_watch(access.scope.as_ref(), decode(args)?, watching).await?;
            encode(json!({"watching":watching,"was_watching":was_watching}), None)
        },
        | "link_pull_request" => encode(
            json!({"already_linked":jobs.set_pull_request_link(access.scope.as_ref(),decode(args)?,true).await?}),
            None,
        ),
        | "unlink_pull_request" => encode(
            json!({"was_linked":jobs.set_pull_request_link(access.scope.as_ref(),decode(args)?,false).await?}),
            None,
        ),
        | "list_thread_pull_requests" => encode(json!({"items":jobs.pull_request_links(access.scope.as_ref(),decode(args)?).await?}), None),
        | _ => Err("Unknown PR tool".into()),
    }
}
