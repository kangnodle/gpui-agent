use serde_json::Value;

// A schema allowlist also removes unknown keys, whose names may contain
// secrets.
pub fn redact(value: &Value, key: &str) -> Value {
    match value {
        | Value::Object(fields) => fields
            .iter()
            .filter(|(name, _)| FIELDS.contains(&name.as_str()))
            .map(|(name, value)| (name.clone(), redact(value, name)))
            .collect(),
        | Value::Array(values) => values.iter().map(|value| redact(value, key)).collect(),
        | Value::String(text) => {
            let safe = (key == "method" && METHODS.contains(&text.as_str()))
                || (key == "jsonrpc" && text == "2.0")
                || (key == "name" && text == "antigravity-acp")
                || (key == "id" && text == "oauth-personal");
            if safe { value.clone() } else { Value::String("[redacted]".into()) }
        },
        | _ => value.clone(),
    }
}

const METHODS: &[&str] = &["initialize", "initialized", "account/read", "authenticate", "session/new"];
const FIELDS: &[&str] = &[
    "jsonrpc",
    "id",
    "method",
    "params",
    "result",
    "error",
    "code",
    "message",
    "data",
    "clientInfo",
    "name",
    "title",
    "version",
    "capabilities",
    "protocolVersion",
    "userAgent",
    "account",
    "refreshToken",
    "email",
    "type",
    "loggedIn",
    "authMethod",
    "authMethods",
    "methodId",
    "agentInfo",
    "agentCapabilities",
    "mcpCapabilities",
    "stdio",
    "promptCapabilities",
    "image",
    "embeddedContext",
    "auth",
    "logout",
    "sessionCapabilities",
    "resume",
    "fs",
    "readTextFile",
    "writeTextFile",
    "terminal",
    "cwd",
    "mcpServers",
    "sessionId",
    "models",
    "availableModels",
    "currentModelId",
    "modelId",
    "description",
    "configOptions",
    "category",
    "options",
    "value",
    "label",
];
