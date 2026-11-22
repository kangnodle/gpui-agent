use super::*;

pub(super) fn tools() -> Vec<ToolDefinition> {
    let id = json!({"type":"integer","minimum":1});
    vec![
        ToolDefinition {
            name: "get_pending_questions",
            description: "Read the currently pending native user questions for a job in your scope, including request_id, question IDs, choices and multi_select. pending=null means no live question; tool permission requests are excluded. Reads do not answer or consume the request. Cancelled/ended/restarted work has no live question; inspect get_job for its state.",
            schema: json!({"type":"object","properties":{"id":id},"required":["id"],"additionalProperties":false}),
            read_only: true,
        },
        ToolDefinition {
            name: "answer_questions",
            description: "Relay a user's answer or skip to a live native question. Operator credential only; unavailable to scoped agent sessions. Use the exact job id and request_id from get_pending_questions. Supply an answers map with every question ID and nonblank string arrays, respecting multi_select, or cancelled to skip the question. Invalid/stale/duplicate replies are rejected without consuming a live question. Acceptance is not provider completion; observe get_job/events. Cannot grant or deny tool permissions.",
            schema: json!({"type":"object","properties":{"id":id,"request_id":id,"response":{"oneOf":[{"type":"object","properties":{"type":{"const":"answers"},"answers":{"type":"object","minProperties":1,"maxProperties":16,"additionalProperties":{"type":"array","minItems":1,"maxItems":32,"items":{"type":"string","minLength":1,"maxLength":16384}}}},"required":["type","answers"],"additionalProperties":false},{"type":"object","properties":{"type":{"const":"cancelled"}},"required":["type"],"additionalProperties":false}]}},"required":["id","request_id","response"],"additionalProperties":false}),
            read_only: false,
        },
    ]
}

pub(super) async fn execute(jobs: &gpui_agent_application::Jobs, name: &str, args: Value, access: &Access) -> Result<Executed, ApplicationError> {
    match name {
        | "get_pending_questions" => {
            let args: JobId = decode(args)?;
            let pending = jobs.get_pending_questions(access.scope.as_ref(), args.id).await?;
            encode(json!({"job_id":args.id,"pending":pending}), Some(jobs.job(args.id).await?))
        },
        | "answer_questions" => {
            let request: QuestionAnswerRequest = decode(args)?;
            let job = jobs.job(request.id).await?;
            jobs.answer_questions(access.scope.as_ref(), request).await?;
            encode(json!({"accepted":true}), Some(job))
        },
        | _ => Err("Unknown tool".into()),
    }
}
