use gpui_agent_contracts::BrowserDiagnostic;
use serde_json::Value;

impl super::Browser {
    pub(super) fn record_diagnostic(&mut self, event: &Value) {
        let params = &event["params"];
        let (kind, level, text, url, line) = match event["method"].as_str() {
            | Some("Runtime.consoleAPICalled") => {
                let args = params["args"].as_array().map(Vec::as_slice).unwrap_or_default();
                let mut text = String::new();
                for value in args.iter().take(20) {
                    if !text.is_empty() {
                        text.push(' ');
                    }
                    let value = value
                        .get("value")
                        .map(|value| value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string()))
                        .unwrap_or_else(|| value["description"].as_str().unwrap_or("[object]").to_owned());
                    text.extend(value.chars().take(2001));
                    if text.chars().count() > 2000 {
                        break;
                    }
                }
                if args.len() > 20 {
                    self.diagnostics.truncated = true;
                }
                let frame = &params["stackTrace"]["callFrames"][0];
                ("console", params["type"].as_str().unwrap_or("log"), text, &frame["url"], &frame["lineNumber"])
            },
            | Some("Runtime.exceptionThrown") => {
                let details = &params["exceptionDetails"];
                let text = details["exception"]["description"]
                    .as_str()
                    .or_else(|| details["text"].as_str())
                    .unwrap_or("Unhandled JavaScript error");
                ("exception", "error", text.to_owned(), &details["url"], &details["lineNumber"])
            },
            | Some("Log.entryAdded") => {
                let entry = &params["entry"];
                (
                    "log",
                    entry["level"].as_str().unwrap_or("info"),
                    entry["text"].as_str().unwrap_or_default().to_owned(),
                    &entry["url"],
                    &entry["lineNumber"],
                )
            },
            | _ => return,
        };
        let url = url.as_str().unwrap_or_default();
        self.diagnostics.truncated |= text.chars().count() > 2000 || url.chars().count() > 512;
        if self.diagnostics.entries.len() == 100 {
            self.diagnostics.entries.remove(0);
            self.diagnostics.truncated = true;
        }
        self.diagnostics.entries.push(BrowserDiagnostic {
            kind: kind.into(),
            level: level.chars().take(32).collect(),
            text: text.chars().take(2000).collect(),
            url: url.chars().take(512).collect(),
            line: line.as_u64().map(|line| line.saturating_add(1)),
        });
    }
}
