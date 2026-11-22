use super::*;

pub(super) fn schema() -> Value {
    let name = json!({"type":"string","minLength":1,"maxLength":255});
    json!({"type":"array","maxItems":8,"items":{"oneOf":[
        {"type":"object","properties":{"type":{"const":"text"},"name":name,"text":{"type":"string","maxLength":262144},"source":{"oneOf":[{"type":"null"},{"type":"object","properties":{"path":{"type":"string","minLength":1,"maxLength":4096},"start_line":{"type":["integer","null"],"minimum":1},"end_line":{"type":["integer","null"],"minimum":1}},"required":["path"],"additionalProperties":false}]}},"required":["type","name","text"],"additionalProperties":false},
        {"type":"object","properties":{"type":{"const":"image"},"name":name,"mime_type":{"enum":["image/png","image/jpeg","image/webp"]},"data":{"type":"string","maxLength":2796204,"description":"Standard base64 image bytes, without a data: URL prefix."}},"required":["type","name","mime_type","data"],"additionalProperties":false}
    ]}})
}

pub(super) fn ids_schema() -> Value { json!({"type":"array","maxItems":8,"items":{"type":"integer","minimum":1}}) }

pub(super) fn tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "attachment_upload",
            description: "Upload one UTF-8 text or PNG/JPEG/WebP image attachment into a durable provider/account/workspace scope. Returns only id/name/kind/bytes, never echoes content. Identical uploads in the same scope reuse the same ID. Text <=256 KiB, image <=2 MiB; 16 uploads per scope and 128 per host. Pass returned IDs as attachment_ids to conversation_send or enqueue_job. Uploading alone does not send to a provider. Delete unused uploads with attachment_delete.",
            schema: conversations::schema(json!({"attachment":schema()["items"]}), &["attachment"]),
            read_only: false,
        },
        ToolDefinition {
            name: "attachment_from_workspace",
            description: "Snapshot one relative workspace UTF-8 file (optionally a one-based line range) or PNG/JPEG/WebP image into a scoped durable upload. Images cannot use line ranges. Returns id/name/kind/bytes without content. The path cannot traverse or escape the workspace, access Git metadata or follow a link outside it. File edits after upload do not change the snapshot. Use attachment_ids with a message to transmit it; uploading alone does not send a message.",
            schema: conversations::schema(
                json!({"path":{"type":"string","minLength":1,"maxLength":4096},"start_line":{"type":"integer","minimum":1},"end_line":{"type":"integer","minimum":1}}),
                &["path"],
            ),
            read_only: false,
        },
        ToolDefinition {
            name: "attachment_list",
            description: "List metadata for up to 16 durable uploads in one provider/account/workspace. Contents, image base64 and source paths are omitted. Scoped sessions cannot select another provider/account/workspace. Uploads persist across host restarts until explicitly deleted; accepted jobs own independent copies.",
            schema: conversations::schema(json!({}), &[]),
            read_only: true,
        },
        ToolDefinition {
            name: "attachment_delete",
            description: "Delete a staged upload in the selected scope. Does not alter accepted jobs, history or an identical command retry. Missing or foreign IDs are rejected. A fresh command cannot use a deleted ID; reupload to get a new ID. Repeated deletion returns not found.",
            schema: conversations::schema(json!({"id":{"type":"integer","minimum":1}}), &["id"]),
            read_only: false,
        },
    ]
}

pub(super) async fn execute(jobs: &gpui_agent_application::Jobs, name: &str, args: Value, access: &Access) -> Result<Executed, ApplicationError> {
    match name {
        | "attachment_upload" => encode(jobs.upload_attachment(access.scope.as_ref(), decode(args)?).await?, None),
        | "attachment_from_workspace" => encode(jobs.attachment_from_workspace(access.scope.as_ref(), decode(args)?).await?, None),
        | "attachment_list" => encode(json!({"items":jobs.uploaded_attachments(access.scope.as_ref(),decode(args)?).await?}), None),
        | "attachment_delete" => {
            jobs.delete_attachment(access.scope.as_ref(), decode(args)?).await?;
            encode(json!({"deleted":true}), None)
        },
        | _ => Err("Unknown attachment tool".into()),
    }
}
