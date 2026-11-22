//! Normalize only native tool events; preserve provider details as JSON.
use gpui_agent_domain::{Event,
                        ToolActivity,
                        ToolCall};
use serde_json::{Value,
                 json};
fn string(value: &Value) -> Option<String> { value.as_str().map(str::to_owned) }
fn non_null(value: &Value) -> Option<Value> { (!value.is_null()).then(|| value.clone()) }
fn call(id: &Value) -> Option<ToolCall> {
    Some(ToolCall {
        id: id.as_str()?.into(),
        title: None,
        status: None,
        input: None,
        output: None,
        output_delta: None,
        activity: vec![],
    })
}
pub fn codex(message: &Value) -> Option<Event> {
    let params = &message["params"];
    let mut tool = match message["method"].as_str()? {
        | "item/started" | "item/completed" => {
            let item = &params["item"];
            let kind = item["type"].as_str()?;
            if !matches!(
                kind,
                "commandExecution"
                    | "fileChange"
                    | "mcpToolCall"
                    | "dynamicToolCall"
                    | "webSearch"
                    | "imageGeneration"
                    | "imageView"
                    | "contextCompaction"
                    | "plan"
                    | "collabAgentToolCall"
                    | "subAgentActivity"
            ) {
                return None;
            }
            let mut tool = call(&item["id"])?;
            tool.title = Some(item["command"].as_str().or(item["tool"].as_str()).unwrap_or(kind).into());
            tool.input = non_null(&item["changes"]).or_else(|| non_null(&item["arguments"]));
            tool.output = non_null(&item["aggregatedOutput"])
                .or_else(|| non_null(&item["result"]))
                .or_else(|| non_null(&item["error"]));
            tool.status = Some(
                item["status"]
                    .as_str()
                    .unwrap_or(if message["method"] == "item/completed" { "completed" } else { "in_progress" })
                    .into(),
            );
            if !item["exitCode"].is_null() {
                tool.output = Some(json!({"output":tool.output,"exit_code":item["exitCode"]}));
            }
            if kind == "plan" {
                tool.output = non_null(&item["text"]);
            } else if matches!(kind, "collabAgentToolCall" | "subAgentActivity" | "imageView" | "contextCompaction") {
                tool.output = Some(item.clone());
            }
            tool
        },
        | "turn/plan/updated" => {
            let mut tool = call(&json!(format!("plan:{}", params["turnId"].as_str()?)))?;
            tool.title = Some("update_plan".into());
            tool.output = Some(params.clone());
            tool.status = Some(
                if params["plan"].as_array()?.iter().all(|step| step["status"] == "completed") {
                    "completed"
                } else {
                    "in_progress"
                }
                .into(),
            );
            tool
        },
        | "item/plan/delta" => {
            let mut tool = call(&params["itemId"])?;
            tool.title = Some("plan".into());
            tool.status = Some("in_progress".into());
            tool.output_delta = string(&params["delta"]);
            tool
        },
        | "item/fileChange/patchUpdated" => {
            let mut tool = call(&params["itemId"])?;
            tool.input = Some(params["changes"].as_array()?.clone().into());
            tool
        },
        | "item/mcpToolCall/progress" => {
            let mut tool = call(&params["itemId"])?;
            tool.activity.push(ToolActivity::Progress {
                message: params["message"].as_str()?.into(),
            });
            tool
        },
        | "item/commandExecution/terminalInteraction" => {
            let mut tool = call(&params["itemId"])?;
            tool.activity.push(ToolActivity::TerminalInput {
                process_id: params["processId"].as_str()?.into(),
                stdin: params["stdin"].as_str()?.into(),
            });
            tool
        },
        | "item/commandExecution/outputDelta" => {
            let mut tool = call(&params["itemId"])?;
            tool.output_delta = string(&params["delta"]);
            tool
        },
        | _ => return None,
    };
    if tool.status.as_deref() == Some("inProgress") {
        tool.status = Some("in_progress".into());
    }
    Some(Event::Tool {
        call: tool,
    })
}
pub fn acp(update: &Value) -> Option<Event> {
    if !matches!(update["sessionUpdate"].as_str()?, "tool_call" | "tool_call_update") {
        return None;
    }
    let mut tool = call(&update["toolCallId"])?;
    tool.title = string(&update["title"]);
    tool.status = string(&update["status"]);
    tool.input = non_null(&update["rawInput"]);
    tool.output = non_null(&update["rawOutput"]).or_else(|| non_null(&update["content"]));
    Some(Event::Tool {
        call: tool,
    })
}
pub fn claude(message: &Value) -> Vec<Event> {
    let mut events = Vec::new();
    if message["type"] == "stream_event" && message["event"]["type"] == "content_block_start" {
        let block = &message["event"]["content_block"];
        if block["type"] == "tool_use"
            && let Some(mut tool) = call(&block["id"])
        {
            tool.title = string(&block["name"]);
            tool.status = Some("pending".into());
            events.push(Event::Tool {
                call: tool,
            });
        }
    }
    if matches!(message["type"].as_str(), Some("assistant" | "user")) {
        for block in message["message"]["content"].as_array().into_iter().flatten() {
            if block["type"] == "tool_use"
                && let Some(mut tool) = call(&block["id"])
            {
                tool.title = string(&block["name"]);
                tool.input = non_null(&block["input"]);
                tool.status = Some("in_progress".into());
                events.push(Event::Tool {
                    call: tool,
                });
            } else if block["type"] == "tool_result"
                && let Some(mut tool) = call(&block["tool_use_id"])
            {
                tool.output = non_null(&block["content"]);
                tool.status = Some(if block["is_error"] == true { "failed" } else { "completed" }.into());
                events.push(Event::Tool {
                    call: tool,
                });
            }
        }
    }
    events
}
