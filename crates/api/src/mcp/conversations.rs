use super::*;

pub(super) fn schema(mut properties: Value, fields: &[&str]) -> Value {
    properties["provider"] = json!({"type":"string","enum":["claude","codex","antigravity"]});
    properties["account"] = json!({"type":"string","description":"Defaults to the calling session account, or default for an operator."});
    properties["workspace"] = json!({"type":"integer","minimum":1});
    let mut required = vec!["provider"];
    required.extend_from_slice(fields);
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}

pub(super) fn tools() -> Vec<ToolDefinition> {
    let list = json!({"query":{"type":"string","maxLength":256},"archived":{"type":"string","enum":["active","archived","all"],"default":"active"},"offset":{"type":"integer","minimum":0,"default":0},"limit":{"type":"integer","minimum":1,"maximum":50,"default":20}});
    vec![
        ToolDefinition {
            name: "conversation_list",
            description: "List saved conversations in one provider/account/workspace. Active conversations by default; archived=all includes archived ones. Uses live offset pages; ordering can change as conversations update.",
            schema: schema(list.clone(), &[]),
            read_only: true,
        },
        ToolDefinition {
            name: "conversation_search",
            description: "Search saved conversation titles and message text using literal substring matching. query must be nonempty, at most 256 characters. Returns bounded message previews and offset pages; archived=all includes archived history.",
            schema: schema(list, &["query"]),
            read_only: true,
        },
        ToolDefinition {
            name: "conversation_read",
            description: "Read saved messages with a budget of 16000 Unicode characters and 1–20 messages per page. Pass next_cursor back as cursor to continue, including within a long message. Native session IDs, tool activity and attachment bytes are omitted; attachment_count indicates attachments. The saved model is included.",
            schema: schema(
                json!({"id":{"type":"integer","minimum":1},"cursor":{"type":"object","properties":{"message":{"type":"integer","minimum":0},"text":{"type":"integer","minimum":0}},"required":["message","text"],"additionalProperties":false},"limit":{"type":"integer","minimum":1,"maximum":20,"default":20}}),
                &["id"],
            ),
            read_only: true,
        },
        ToolDefinition {
            name: "conversation_rename",
            description: "Rename a saved conversation through the existing history service. Active responses reject renaming. Repeating the same title is safe.",
            schema: schema(
                json!({"id":{"type":"integer","minimum":1},"title":{"type":"string","minLength":1,"maxLength":100}}),
                &["id", "title"],
            ),
            read_only: false,
        },
        ToolDefinition {
            name: "conversation_organize",
            description: "Set pinned and/or archived state on a saved conversation. Omitted fields are preserved; false unpins/unarchives. This does not stop running work. Existing message delivery can unarchive a conversation.",
            schema: schema(
                json!({"id":{"type":"integer","minimum":1},"pinned":{"type":"boolean"},"archived":{"type":"boolean"}}),
                &["id"],
            ),
            read_only: false,
        },
        ToolDefinition {
            name: "conversation_send",
            description: "Queue a durable message in a saved conversation, or omit id to start a new conversation when the job executes. Requires a unique command_id; identical retries return the original job. Accepts inline attachments and scoped attachment_ids from upload tools (combined maximum 8/4 MiB); text may be empty with attachments. Uploaded bytes are copied at admission, before later upload deletion. Returns acceptance, not completion. Active turns are followed by queued turns, not live steering. Optional model selects this turn's model; omission uses the provider default, except the first independent fork turn inherits its saved model. Tool approvals still use the existing provider/user flow.",
            schema: schema(
                json!({"id":{"type":"integer","minimum":1},"text":{"type":"string","maxLength":262144},"skills":{"type":"array","maxItems":8,"items":{"type":"object","properties":{"id":{"type":"string","minLength":1,"maxLength":4101}},"required":["id"],"additionalProperties":false}},"attachments":attachments::schema(),"attachment_ids":attachments::ids_schema(),"model":{"type":"string","minLength":1,"maxLength":512},"command_id":{"type":"string","minLength":1,"maxLength":128,"pattern":"^[A-Za-z0-9_.:-]+$"}}),
                &["text", "command_id"],
            ),
            read_only: false,
        },
        ToolDefinition {
            name: "conversation_wait",
            description: "Wait up to timeout_ms (0–10000, default 1000) for a durable conversation job to end. Omit job_id to select the latest job once at call time. Timeout does not cancel work. job=null means no durable job exists. Direct CLI/streaming turns outside the durable queue are not tracked. Same-provider/account queued work starts after the parent turn ends.",
            schema: schema(
                json!({"id":{"type":"integer","minimum":1},"job_id":{"type":"integer","minimum":1},"timeout_ms":{"type":"integer","minimum":0,"maximum":10000,"default":1000}}),
                &["id"],
            ),
            read_only: true,
        },
        ToolDefinition {
            name: "conversation_interrupt",
            description: "Request interruption of one durable conversation job. Omit job_id to prefer the running/waiting job, then the newest queued job. Specify job_id to cancel a particular queued turn. Other queued turns remain queued. Repeated interruption of the same terminal job preserves its result. Observe conversation_wait for completion; idle conversations return job=null.",
            schema: schema(json!({"id":{"type":"integer","minimum":1},"job_id":{"type":"integer","minimum":1}}), &["id"]),
            read_only: false,
        },
        ToolDefinition {
            name: "conversation_fork",
            description: "Fork saved history through a completed assistant message into an idle conversation with an independent native session. Omit through_message_id for the current last message; unfinished boundaries are rejected. Copies messages, attachments, tool results and model; permissions and native session IDs are excluded. Requires command_id; identical retries return the original transfer even after history changes. Pass returned target_id as conversation_send id to start work; its first turn inherits the saved model unless overridden.",
            schema: schema(
                json!({"id":{"type":"integer","minimum":1},"through_message_id":{"type":"integer","minimum":1},"title":{"type":"string","minLength":1,"maxLength":100},"command_id":{"type":"string","minLength":1,"maxLength":128,"pattern":"^[A-Za-z0-9_.:-]+$"}}),
                &["id", "command_id"],
            ),
            read_only: false,
        },
        ToolDefinition {
            name: "conversation_merge_back",
            description: "Save a bounded snapshot of branch work for an ancestor conversation in this provider/account/workspace. Includes only messages after that ancestor's fork boundary, through a completed assistant message. Does not rewrite history or start/steer a turn: pending context attaches to the next accepted turn. Requires a unique command_id; retry identical requests with the same ID. Existing attachment limits also apply to combined pending context.",
            schema: schema(
                json!({"source_id":{"type":"integer","minimum":1},"target_id":{"type":"integer","minimum":1},"through_message_id":{"type":"integer","minimum":1},"command_id":{"type":"string","minLength":1,"maxLength":128,"pattern":"^[A-Za-z0-9_.:-]+$"}}),
                &["source_id", "target_id", "command_id"],
            ),
            read_only: false,
        },
        ToolDefinition {
            name: "conversation_transfers",
            description: "List up to 50 fork/merge records involving a conversation after an ID cursor. pending means saved; attached means committed into an admitted turn's input, not provider completion. Native session IDs and snapshot bytes are never returned. Echo next_after for more records.",
            schema: schema(
                json!({"id":{"type":"integer","minimum":1},"after":{"type":"integer","minimum":0,"default":0}}),
                &["id"],
            ),
            read_only: true,
        },
    ]
}

pub(super) async fn execute(jobs: &gpui_agent_application::Jobs, name: &str, args: Value, access: &Access) -> Result<Executed, ApplicationError> {
    match name {
        | "conversation_list" | "conversation_search" => {
            let request: ConversationListRequest = decode(args)?;
            if name == "conversation_search" && request.query.as_ref().is_none_or(|query| query.trim().is_empty()) {
                return Err("Search query must be nonempty".into());
            }
            encode(jobs.conversation_list(access.scope.as_ref(), request).await?, None)
        },
        | "conversation_read" => encode(jobs.conversation_read(access.scope.as_ref(), decode(args)?).await?, None),
        | "conversation_rename" => {
            jobs.conversation_rename(access.scope.as_ref(), decode(args)?).await?;
            encode(json!({"updated":true}), None)
        },
        | "conversation_organize" => {
            jobs.conversation_organize(access.scope.as_ref(), decode(args)?).await?;
            encode(json!({"updated":true}), None)
        },
        | "conversation_send" => {
            let job = jobs.conversation_send(access.scope.as_ref(), decode(args)?).await?;
            encode(&job, Some(job.clone()))
        },
        | "conversation_wait" => {
            let result = jobs.conversation_wait(access.scope.as_ref(), decode(args)?).await?;
            let job = result.job.clone();
            encode(result, job)
        },
        | "conversation_interrupt" => {
            let job = jobs.conversation_interrupt(access.scope.as_ref(), decode(args)?).await?;
            encode(json!({"job":job,"accepted":true}), job)
        },
        | "conversation_fork" => encode(jobs.conversation_fork(access.scope.as_ref(), decode(args)?).await?, None),
        | "conversation_merge_back" => encode(jobs.conversation_merge_back(access.scope.as_ref(), decode(args)?).await?, None),
        | "conversation_transfers" => {
            let items = jobs.conversation_transfers(access.scope.as_ref(), decode(args)?).await?;
            let next_after = if items.len() == 50 { items.last().map(|item| item.id) } else { None };
            encode(json!({"items":items,"next_after":next_after}), None)
        },
        | _ => Err("Unknown tool".into()),
    }
}
