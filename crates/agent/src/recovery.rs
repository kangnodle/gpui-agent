use gpui_agent_domain::{Event,
                        RuntimeStatus};
use serde_json::Value;
pub(crate) fn codex(message: &Value) -> Option<Event> {
    let params = &message["params"];
    let (status, message, code) = match message["method"].as_str()? {
        | "warning" => (RuntimeStatus::Warning, params["message"].as_str()?, None),
        | "error" => {
            let error = &params["error"];
            let info = &error["codexErrorInfo"];
            let code = info.as_str().map(str::to_owned).or_else(|| info.as_object()?.keys().next().cloned());
            let status = if params["willRetry"] == true {
                RuntimeStatus::Retrying
            } else if code.as_deref() == Some("usageLimitExceeded") {
                RuntimeStatus::UsageLimited
            } else {
                RuntimeStatus::Error
            };
            (status, error["message"].as_str()?, code)
        },
        | _ => return None,
    };
    Some(Event::RuntimeStatus {
        status,
        message: message.into(),
        code,
    })
}
