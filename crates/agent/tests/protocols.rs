#[path = "support/oauth_fixture.rs"]
pub(crate) mod oauth_fixture;
use anyhow::{Context,
             Result,
             ensure};
use gpui_agent::{Agent,
                 Config,
                 Event,
                 Prompt,
                 Provider};
use serde_json::{Value,
                 json};
use std::{fs,
          io::{self,
               BufRead,
               Write},
          path::PathBuf,
          time::Duration};

const URL: &str = "https://accounts.google.com/o/oauth2/v2/auth?state=fixture&response_type=code&redirect_uri=http%3A%2F%2F127.0.0.1%3A51234%2F";

fn emit(value: Value) {
    println!("{value}");
    io::stdout().flush().unwrap();
}

fn allows(scenario: &str) -> bool {
    scenario.starts_with("interactive-permission-")
        || matches!(
            scenario,
            "allow" | "interactive-allow" | "interactive-question" | "interactive-question-cancel" | "interactive-running" | "interactive-failed"
        )
}
fn emit_tool(provider: &str, status: &str, output: Option<&str>) {
    match provider {
        | "claude" =>
            if let Some(output) = output {
                emit(
                    json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool-1","content":output,"is_error":status == "failed"}]}}),
                );
            } else {
                emit(json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"tool-1","name":"Bash","input":{"command":"fixture-command"}}]}}));
            },
        | "codex" => emit(
            json!({"method":if output.is_some() { "item/completed" } else { "item/started" },"params":{"threadId":"codex-session","turnId":"turn1","item":{"id":"tool-1","type":"commandExecution","command":"fixture-command","status":status,"aggregatedOutput":output,"exitCode":if status == "failed" { Some(1) } else { None }}}}),
        ),
        | _ => emit(
            json!({"method":"session/update","params":{"sessionId":"agy-session","update":{"sessionUpdate":if output.is_some() { "tool_call_update" } else { "tool_call" },"toolCallId":"tool-1","title":"fixture-command","status":status,"rawOutput":output}}}),
        ),
    }
}

pub fn fixture(mut spec: Value) -> Result<()> {
    if spec["provider"] == "auto" {
        spec["provider"] = json!(match std::env::args().nth(1).as_deref() {
            | Some("auth" | "-p" | "--help") => "claude",
            | Some("app-server") => "codex",
            | _ => "antigravity",
        });
    }
    let provider = spec["provider"].as_str().unwrap();
    let scenario = spec["scenario"].as_str().unwrap();
    let key = match provider {
        | "claude" => "CLAUDE_CONFIG_DIR",
        | "codex" => "CODEX_HOME",
        | _ => "GEMINI_HOME",
    };
    let home = PathBuf::from(std::env::var_os(key).unwrap());
    ensure!(home.is_absolute(), "home was not normalized");
    let token = if provider == "antigravity" {
        home.join("antigravity-acp/acp_token.json")
    } else {
        home.join("fixture-auth")
    };
    let args: Vec<_> = std::env::args().skip(1).collect();
    if provider == "claude" {
        if args.first().map(String::as_str) == Some("auth") {
            match args.get(1).map(String::as_str) {
                | Some("login") => fs::write(&token, "fixture")?,
                | Some("logout") =>
                    if token.exists() {
                        fs::remove_file(&token)?;
                    },
                | Some("status") => {
                    if scenario == "status-broken" {
                        emit(json!({"loggedIn":true}));
                        std::process::exit(2);
                    }
                    emit(json!({"loggedIn":token.exists(),"email":"fixture@example.invalid","authMethod":"claude.ai"}));
                    if !token.exists() {
                        std::process::exit(1);
                    }
                },
                | _ => anyhow::bail!("unexpected auth command"),
            }
            return Ok(());
        }
        let mcp_index = args.iter().position(|arg| arg == "--mcp-config").context("MCP config missing")?;
        let mcp_argument = &args[mcp_index + 1];
        let mcp_config: Value = if mcp_argument.starts_with('{') {
            serde_json::from_str(mcp_argument)?
        } else {
            serde_json::from_reader(fs::File::open(mcp_argument)?)?
        };
        if mcp_config["mcpServers"]["remote"].is_object() && scenario != "oauth-mcp" {
            let remote = &mcp_config["mcpServers"]["remote"];
            ensure!(remote["type"] == "http" && remote["url"] == "https://mcp.example.invalid/tools");
            ensure!(remote["headers"]["Authorization"] == format!("Bearer {}", std::env::var("HTTP_MCP_TEST_TOKEN")?));
            ensure!(remote["headers"]["X-Tenant"] == std::env::var("HTTP_MCP_TEST_TENANT")?);
            ensure!(args.iter().all(|argument| !argument.contains(&std::env::var("HTTP_MCP_TEST_TOKEN").unwrap())));
            ensure!(!mcp_argument.starts_with('{'), "HTTP credentials must not be placed in process arguments");
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(home.join("claude-http-starts.jsonl"))?
                .write_all(format!("{}\n", json!({"resume":args.iter().any(|arg|arg=="--resume"),"config_path":mcp_argument})).as_bytes())?;
        }
        if scenario == "oauth-mcp" {
            ensure!(
                mcp_config["mcpServers"]["remote"]["headers"]["Authorization"] == "Bearer fixture-oauth-access",
                "OAuth credential missing from Claude native configuration"
            );
            ensure!(args.iter().all(|arg| !arg.contains("fixture-oauth-access")));
        }
        if mcp_config["mcpServers"]["gpui-agent-session"].is_object() {
            ensure!(std::env::var("GPUI_AGENT_MCP_TOKEN")?.starts_with("session_"));
            ensure!(mcp_config["mcpServers"]["gpui-agent-session"]["args"] == json!(["--mcp-stdio"]));
            if spec["probeMcp"] == true {
                probe_session_mcp(&mcp_config["mcpServers"]["gpui-agent-session"], provider)?;
            }
        }
        let mut recorded = mcp_config.clone();
        redact_session_credentials(&mut recorded);
        fs::write(home.join("claude-mcp.json"), recorded.to_string())?;
        if home.join("mcp.json").exists() && scenario != "mcp-selection" && !args.iter().any(|arg| arg == "--safe-mode") {
            let config = &mcp_config;
            let expected: Value = serde_json::from_slice(&fs::read(home.join("mcp.json"))?)?;
            ensure!(config["mcpServers"]["fixture"]["command"] == expected[0]["command"]);
            ensure!(config["mcpServers"]["fixture"]["args"] == expected[0]["args"]);
        }
        let stdin = io::stdin();
        let mut lines = stdin.lock().lines();
        let init: Value = serde_json::from_str(&lines.next().unwrap()?)?;
        ensure!(init["request"]["subtype"] == "initialize");
        emit(json!({"type":"control_response","response":{"subtype":"success","request_id":init["request_id"],"response":{}}}));
        let user: Value = serde_json::from_str(&lines.next().unwrap()?)?;
        fs::write(home.join("claude-input.json"), serde_json::to_vec(&user)?)?;
        let content = &user["message"]["content"];
        let prompt = content.as_str().or_else(|| content[0]["text"].as_str()).unwrap();
        if scenario.starts_with("attachments") {
            check_attachments(content, provider, scenario)?;
        }
        if scenario == "guidance" {
            check_guidance(prompt)?;
        }
        if scenario != "guidance" && !scenario.starts_with("interactive-") && !scenario.starts_with("attachments") {
            ensure!(
                prompt.split_once("\n\nUser request:\n").map_or(prompt, |(_, text)| text) == "안녕 $() `test`\nsecond line",
                "prompt changed at process boundary"
            );
        }
        let mode = spec["mode"].as_str().unwrap_or("default");
        ensure!(args.windows(2).any(|pair| pair == ["--permission-mode", mode]));
        ensure!(args.iter().any(|arg| arg == "--allow-dangerously-skip-permissions"));
        ensure!(args.windows(2).any(|pair| pair == ["--permission-prompt-tool", "stdio"]));
        emit_tool(provider, "in_progress", None);
        emit(
            json!({"type":"control_request","request_id":"permission","request":{"subtype":"can_use_tool","tool_name":if spec["computerPermissions"] == true { "mcp__gpui-agent-session__computer_capture" } else { "Bash" },"tool_use_id":"tool-1","input":{"command":"fixture-command"},"permission_suggestions":[{"type":"addRules","rules":[{"toolName":"Write"}],"behavior":"allow","destination":"session"},{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"cargo test:*"}],"behavior":"allow","destination":"localSettings"}]}}),
        );
        let reply: Value = serde_json::from_str(&lines.next().unwrap()?)?;
        ensure!(reply["response"]["request_id"] == "permission");
        ensure!(reply["response"]["response"]["behavior"] == if allows(scenario) { "allow" } else { "deny" });
        if scenario == "interactive-permission-mode" {
            ensure!(reply["response"]["response"]["updatedPermissions"] == json!([{"type":"setMode","mode":mode,"destination":"session"}]));
            ensure!(reply["response"]["response"]["updatedInput"] == json!({"command":"fixture-command"}));
        }
        if scenario == "interactive-permission-suggestion" {
            ensure!(
                reply["response"]["response"]["updatedPermissions"]
                    == json!([{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"cargo test:*"}],"behavior":"allow","destination":"localSettings"}])
            );
        }
        if scenario.starts_with("interactive-question") {
            emit(
                json!({"type":"control_request","request_id":"question","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"Choose?","options":[{"label":"A"},{"label":"B"}],"multiSelect":false}]}}}),
            );
            let reply: Value = serde_json::from_str(&lines.next().unwrap()?)?;
            if scenario == "interactive-question-cancel" {
                ensure!(reply["response"]["response"]["behavior"] == "deny");
            } else {
                ensure!(reply["response"]["response"]["updatedInput"]["answers"]["Choose?"] == "B");
            }
        }
        if scenario == "interactive-running" {
            emit_tool(provider, "in_progress", None);
            std::thread::park();
            unreachable!();
        }
        emit_tool(
            provider,
            if scenario == "interactive-failed" {
                "failed"
            } else if allows(scenario) {
                "completed"
            } else {
                "failed"
            },
            Some("fixture result 🦀"),
        );
        emit(json!({"type":"system","subtype":"init","session_id":"claude-session"}));
        emit(json!({"type":"stream_event","event":{"delta":{"type":"text_delta","text":"Hello "}}}));
        emit(json!({"type":"stream_event","event":{"delta":{"type":"text_delta","text":"세계"}}}));
        emit(json!({"type":"assistant","message":{"content":[{"type":"text","text":"Hello 세계"}]}}));
        emit(
            json!({"type":"result","subtype":if scenario == "failed" { "error_max_turns" } else { "success" },
            "is_error":scenario == "failed","result":"Hello 세계","session_id":"claude-session","usage":{"input_tokens":10,"output_tokens":2,"cache_read_input_tokens":3,"cache_creation_input_tokens":1},"total_cost_usd":0.001}),
        );
        if scenario == "exit-failed" {
            std::process::exit(3);
        }
        return Ok(());
    }
    if provider == "antigravity" {
        ensure!(std::env::var("AGY_ACP_FORCE_FILE_STORAGE")? == "1");
        ensure!(std::env::var("BROWSER")?.contains("__auth-browser"));
        ensure!(std::env::var_os("ANTIGRAVITY_HARNESS_PATH").is_some());
    }
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    while let Some(line) = lines.next() {
        let message: Value = serde_json::from_str(&line?)?;
        let mut recorded = message.clone();
        redact_session_credentials(&mut recorded);
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(home.join("requests.jsonl"))?
            .write_all(format!("{recorded}\n").as_bytes())?;
        let Some(method) = message["method"].as_str() else {
            continue;
        };
        if message.get("id").is_none() {
            continue;
        }
        if provider == "antigravity" {
            ensure!(message["jsonrpc"] == "2.0");
        }
        let id = &message["id"];
        let params = &message["params"];
        if let Some(servers) = params["mcpServers"].as_array() {
            for server in servers.iter().filter(|server| server["name"] == "gpui-agent-session") {
                if spec["probeMcp"] == true {
                    probe_session_mcp(server, provider)?;
                }
                if server["type"] == "http" {
                    ensure!(server["headers"].as_array().context("session headers")?.iter().any(|entry| {
                        entry["name"] == "Authorization" && entry["value"].as_str().is_some_and(|value| value.starts_with("Bearer session_"))
                    }));
                } else {
                    ensure!(
                        server["env"]
                            .as_array()
                            .context("session environment")?
                            .iter()
                            .any(|entry| entry["name"] == "GPUI_AGENT_MCP_TOKEN" && entry["value"].as_str().is_some_and(|value| value.starts_with("session_")))
                    );
                }
            }
        }
        let result = match method {
            | "initialize" if scenario == "timeout" => {
                std::thread::park();
                unreachable!();
            },
            | "initialize" if scenario == "bad-json" => {
                println!("secret invalid payload");
                io::stdout().flush()?;
                continue;
            },
            | "initialize" if scenario == "crash" => std::process::exit(9),
            | "initialize" if provider == "codex" => json!({"userAgent":"fixture"}),
            | "initialize" => {
                let mut initialized = json!({"protocolVersion":2,"agentInfo":{"name":"antigravity-acp"},
                    "authMethods":[{"id":"oauth-personal"}],"agentCapabilities":{"mcpCapabilities":{"stdio":scenario != "mcp-unsupported" && scenario != "http-mcp-only","http":matches!(scenario,"http-mcp"|"http-mcp-only"|"oauth-mcp")},"promptCapabilities":{"image":scenario != "attachments-no-images","embeddedContext":scenario != "attachments-fallback"},"auth":{"logout":{}},"sessionCapabilities":{"resume":{}}}});
                if scenario == "http-mcp-v2" {
                    initialized["agentCapabilities"]["mcpCapabilities"] = json!({});
                    initialized["capabilities"] = json!({"session":{"mcp":{"stdio":{},"http":{}}}});
                }
                initialized
            },
            | "account/read" => {
                if home.join("hold-status").exists() {
                    fs::write(home.join("status-ready"), "")?;
                    let deadline = std::time::Instant::now() + Duration::from_secs(15);
                    while !home.join("release-status").exists() {
                        ensure!(std::time::Instant::now() < deadline, "Status fixture was not released");
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    fs::remove_file(home.join("hold-status"))?;
                }
                json!({"account":if token.exists() {json!({"type":"chatgpt","email":"fixture@example.invalid"})} else {Value::Null}})
            },
            | "account/login/start" => {
                fs::write(&token, "fixture")?;
                emit(json!({"method":"account/login/completed","params":{"loginId":"login-fixture","success":true}}));
                if params["type"] == "chatgptDeviceCode" {
                    json!({"loginId":"login-fixture","verificationUrl":"https://auth.openai.com/codex/device","userCode":"ABCD"})
                } else {
                    json!({"loginId":"login-fixture","authUrl":"https://auth.openai.com/authorize?state=fixture"})
                }
            },
            | "account/logout" | "logout" => {
                if token.exists() {
                    fs::remove_file(&token)?;
                }
                json!({})
            },
            | "authenticate" => {
                if !token.exists() || scenario == "expired" {
                    println!("Open the following link to authenticate the ACP server: {URL}");
                    io::stdout().flush()?;
                    if scenario == "expired" {
                        std::thread::park();
                        unreachable!();
                    }
                    fs::write(&token, "fixture")?;
                }
                json!({})
            },
            | "model/list" =>
                if params["cursor"].is_null() {
                    json!({"data":[{"id":"first","displayName":"First"}],"nextCursor":"page2"})
                } else {
                    json!({"data":[{"id":"second","displayName":"Second"}],"nextCursor":null})
                },
            | "thread/start" | "thread/resume" => {
                if params["config"]["mcp_servers.gpui-agent-session.command"].is_string() {
                    ensure!(std::env::var("GPUI_AGENT_MCP_TOKEN")?.starts_with("session_"));
                    ensure!(params["config"]["mcp_servers.gpui-agent-session.env_vars"] == json!(["GPUI_AGENT_MCP_TOKEN", "GPUI_AGENT_MCP_URL"]));
                    if spec["probeMcp"] == true {
                        probe_session_mcp(
                            &json!({"command":params["config"]["mcp_servers.gpui-agent-session.command"],"args":["--mcp-stdio"]}),
                            provider,
                        )?;
                    }
                }
                if scenario == "oauth-mcp" {
                    let variable = params["config"]["mcp_servers.remote.bearer_token_env_var"]
                        .as_str()
                        .context("Missing OAuth bearer reference")?;
                    ensure!(variable.starts_with("GPUI_AGENT_MCP_OAUTH_") && std::env::var(variable)? == "fixture-oauth-access");
                    ensure!(!params.to_string().contains("fixture-oauth-access"));
                }
                if home.join("mcp.json").exists() && !matches!(scenario, "oauth-mcp" | "mcp-selection") {
                    let expected: Value = serde_json::from_slice(&fs::read(home.join("mcp.json"))?)?;
                    ensure!(params["config"]["mcp_servers.fixture.command"] == expected[0]["command"]);
                    ensure!(params["config"]["mcp_servers.fixture.args"] == expected[0]["args"]);
                    if expected.as_array().unwrap().iter().any(|server| server["name"] == "remote") {
                        ensure!(params["config"]["mcp_servers.remote.url"] == "https://mcp.example.invalid/tools");
                        ensure!(params["config"]["mcp_servers.remote.bearer_token_env_var"] == "HTTP_MCP_TEST_TOKEN");
                        ensure!(params["config"]["mcp_servers.remote.env_http_headers"] == json!({"X-Tenant":"HTTP_MCP_TEST_TENANT"}));
                        ensure!(params["config"].get("mcp_servers.remote.command").is_none());
                    }
                }
                let expected_policy = spec
                    .get("policy")
                    .cloned()
                    .unwrap_or_else(|| json!({"approval_policy":"untrusted","sandbox":"read-only"}));
                ensure!(
                    params["approvalPolicy"] == expected_policy["approval_policy"] && params["sandbox"] == expected_policy["sandbox"],
                    "Selected Codex policy must reach both thread start and resume"
                );
                ensure!(
                    std::fs::canonicalize(params["cwd"].as_str().unwrap())? == std::fs::canonicalize(std::env::current_dir()?)?,
                    "Native runtime cwd changed at the process boundary"
                );
                if method == "thread/resume" {
                    ensure!(params["threadId"] == "codex-session");
                }
                json!({"thread":{"id":"codex-session"}})
            },
            | "mcpServerStatus/list" => {
                ensure!(params["threadId"] == "codex-session" && params["serverName"] == "gpui-agent-session" && params["detail"] == "toolsAndAuthOnly");
                if scenario == "missing-host-mcp" {
                    json!({"data":[],"nextCursor":null})
                } else {
                    json!({"data":[{"name":"gpui-agent-session","tools":{"list_jobs":{"name":"list_jobs"}}}],"nextCursor":null})
                }
            },
            | "turn/start" => {
                if scenario == "guidance" {
                    check_guidance(params["input"][0]["text"].as_str().context("Codex text input")?)?;
                }
                if matches!(scenario, "live-input" | "live-input-rejected") {
                    emit(json!({"id":id,"result":{"turn":{"id":"turn1"}}}));
                    emit(json!({"method":"turn/started","params":{"threadId":"codex-session","turn":{"id":"turn1","status":"inProgress"}}}));
                    let steer: Value = serde_json::from_str(&lines.next().context("Missing live input")??)?;
                    ensure!(steer["method"] == "turn/steer" && steer["params"]["threadId"] == "codex-session" && steer["params"]["expectedTurnId"] == "turn1");
                    ensure!(
                        steer["params"]["input"][0]["text"]
                            .as_str()
                            .is_some_and(|text| text.ends_with("Updated instruction"))
                    );
                    emit(json!({"method":"item/agentMessage/delta","params":{"threadId":"codex-session","turnId":"turn1","delta":"Observed steering"}}));
                    if scenario == "live-input-rejected" {
                        emit(json!({"id":steer["id"],"error":{"code":-32000,"message":"Turn no longer accepts steering"}}));
                    } else {
                        emit(json!({"id":steer["id"],"result":{"turnId":"turn1"}}));
                    }
                    emit(json!({"method":"turn/completed","params":{"threadId":"codex-session","turn":{"id":"turn1","status":"completed"}}}));
                    continue;
                }
                if scenario == "progress-events" {
                    emit(
                        json!({"method":"item/mcpToolCall/progress","params":{"threadId":"codex-session","turnId":"old-turn","itemId":"mcp-progress","message":"Before start, old turn"}}),
                    );
                }
                emit(json!({"method":"turn/started","params":{"threadId":"codex-session","turn":{"id":"turn1","status":"inProgress"}}}));
                if scenario == "retry-events" {
                    emit(json!({"method":"error","params":{"threadId":"other","turnId":"turn1","willRetry":true,"error":{"message":"foreign retry"}}}));
                    emit(
                        json!({"method":"error","params":{"threadId":"codex-session","turnId":"turn1","willRetry":true,"error":{"message":"Reconnecting 1/3","codexErrorInfo":"responseStreamConnectionFailed"}}}),
                    );
                    emit(json!({"method":"warning","params":{"threadId":"codex-session","turnId":"turn1","message":"HTTPS fallback"}}));
                }
                if scenario.starts_with("attachments") {
                    check_attachments(&params["input"], provider, scenario)?;
                }
                if scenario == "cancel" {
                    std::thread::park();
                    unreachable!();
                }
                if scenario.starts_with("permissions-") {
                    // Given a native request, when the user decides, then only
                    // the requested turn permissions are returned.
                    let permissions = json!({"network":{"enabled":true},"fileSystem":{"write":["C:/fixture/output"]}});
                    emit(json!({"id":"permissions","method":"item/permissions/requestApproval","params":{
                        "threadId":"codex-session","turnId":"turn1","itemId":"permission-item","cwd":"C:/fixture",
                        "startedAtMs":1,"reason":"Run fixture","permissions":permissions
                    }}));
                    let reply: Value = serde_json::from_str(&lines.next().unwrap()?)?;
                    ensure!(reply["id"] == "permissions");
                    ensure!(
                        reply["result"] == json!({"permissions":if scenario == "permissions-accept" { permissions } else { json!({}) },"scope":"turn"}),
                        "Invalid permission response: {reply}"
                    );
                }
                if scenario.starts_with("elicitation-") {
                    let mut request = json!({"id":"elicit","method":"mcpServer/elicitation/request","params":{
                        "threadId":"codex-session","turnId":"turn1","serverName":"fixture-server","mode":"form",
                        "message":"Configure the fixture","requestedSchema":{"type":"object","properties":{
                            "name":{"type":"string","minLength":1},"count":{"type":"integer","minimum":1},"enabled":{"type":"boolean"}
                        },"required":["name","count"]}
                    }});
                    if scenario.contains("computer-") {
                        if scenario.contains("invalid-id") {
                            request["params"]["itemId"] = json!(17);
                        }
                        request["params"]["serverName"] = json!("gpui-agent-session");
                        request["params"]["message"] = json!("도구 실행 확인 / arbitrary display text");
                        request["params"]["requestedSchema"] = json!({"type":"object","properties":{}});
                        emit(
                            json!({"method":"item/started","params":{"threadId":"codex-session","turnId":"turn1","item":{"id":"computer-item","type":"mcpToolCall","server":"gpui-agent-session","tool":"computer_act"}}}),
                        );
                        if scenario.contains("ambiguous") {
                            emit(
                                json!({"method":"item/started","params":{"threadId":"codex-session","turnId":"turn1","item":{"id":"other-item","type":"mcpToolCall","server":"gpui-agent-session","tool":"cancel_job"}}}),
                            );
                        }
                    }
                    if scenario.contains("url-") {
                        request["params"]["mode"] = json!("url");
                        request["params"]["url"] = json!(if scenario.ends_with("unsafe") {
                            "javascript:alert(1)"
                        } else {
                            "https://example.invalid/verify"
                        });
                        request["params"]["elicitationId"] = json!("verification-1");
                        request["params"].as_object_mut().unwrap().remove("requestedSchema");
                    }
                    emit(request);
                    let reply: Value = serde_json::from_str(&lines.next().unwrap()?)?;
                    let action = scenario.rsplit('-').next().unwrap();
                    ensure!(
                        reply["id"] == "elicit" && reply["result"]["action"] == action,
                        "Invalid elicitation response: {reply}"
                    );
                    ensure!(
                        reply["result"]["content"]
                            == if action == "accept" && scenario.contains("computer-") {
                                json!({})
                            } else if action == "accept" && !scenario.contains("url-") {
                                json!({"name":"fixture 🦀","count":2,"enabled":false})
                            } else {
                                Value::Null
                            }
                    );
                }
                if scenario == "progress-events" {
                    emit(
                        json!({"method":"item/started","params":{"threadId":"codex-session","turnId":"turn1","item":{"type":"commandExecution","id":"terminal-progress","command":"fixture-terminal","status":"inProgress"}}}),
                    );
                    emit(
                        json!({"method":"item/commandExecution/outputDelta","params":{"threadId":"codex-session","turnId":"turn1","itemId":"terminal-progress","delta":"stdout before input"}}),
                    );
                    emit(
                        json!({"method":"item/commandExecution/terminalInteraction","params":{"threadId":"codex-session","turnId":"turn1","itemId":"terminal-progress","processId":"process-42","stdin":"계속\nnext line\n"}}),
                    );
                    emit(
                        json!({"method":"item/started","params":{"threadId":"codex-session","turnId":"turn1","item":{"type":"fileChange","id":"patch-progress","changes":[],"status":"inProgress"}}}),
                    );
                    emit(
                        json!({"method":"item/fileChange/patchUpdated","params":{"threadId":"codex-session","turnId":"turn1","itemId":"patch-progress","changes":[{"path":"src/fixture.rs","kind":{"type":"update","move_path":null},"diff":"-before\n+after\n"}]}}),
                    );
                    emit(
                        json!({"method":"item/started","params":{"threadId":"codex-session","turnId":"turn1","item":{"type":"mcpToolCall","id":"mcp-progress","tool":"lookup","status":"inProgress","arguments":{"query":"fixture"}}}}),
                    );
                    for (thread, turn, message) in [
                        ("codex-session", "turn1", "자료 검색 중"),
                        ("other", "turn1", "Foreign progress"),
                        ("codex-session", "old-turn", "Old progress"),
                    ] {
                        emit(
                            json!({"method":"item/mcpToolCall/progress","params":{"threadId":thread,"turnId":turn,"itemId":"mcp-progress","message":message}}),
                        );
                    }
                    emit(json!({"method":"turn/plan/updated","params":{"threadId":"other","turnId":"wrong","plan":[{"step":"Ignore","status":"pending"}]}}));
                    for status in ["inProgress", "completed"] {
                        emit(
                            json!({"method":if status=="completed"{"item/completed"}else{"item/started"},"params":{"threadId":"codex-session","turnId":"turn1","item":{"type":"imageView","id":"image-view-1","path":"C:/fixture/이미지 preview.png"}}}),
                        );
                        emit(
                            json!({"method":"turn/plan/updated","params":{"threadId":"codex-session","turnId":"turn1","explanation":"Fixture plan","plan":[{"step":"Verify","status":status}]}}),
                        );
                        emit(
                            json!({"method":if status == "completed" { "item/completed" } else { "item/started" },"params":{"threadId":"codex-session","turnId":"turn1","item":{
                                "type":"collabAgentToolCall","id":"collab-1","tool":"spawnAgent","status":status,"senderThreadId":"codex-session",
                                "receiverThreadIds":["child-1"],"prompt":"Verify changes","model":"fixture-model","agentsStates":{"child-1":{"status":"completed","message":"Verified"}}
                            }}}),
                        );
                    }
                    emit(json!({"method":"item/plan/delta","params":{"threadId":"codex-session","turnId":"turn1","itemId":"plan-item","delta":"Draft plan"}}));
                    emit(
                        json!({"method":"item/completed","params":{"threadId":"codex-session","turnId":"turn1","item":{"type":"plan","id":"plan-item","text":"Authoritative plan"}}}),
                    );
                    emit(
                        json!({"method":"item/completed","params":{"threadId":"codex-session","turnId":"turn1","item":{"type":"subAgentActivity","id":"activity","kind":"started","agentThreadId":"child-1","agentPath":"/root/child","model":"fixture-model"}}}),
                    );
                    emit(json!({"id":id,"result":{"turn":{"id":"turn1"}}}));
                    for (method, fields) in [
                        ("item/mcpToolCall/progress", json!({"message":false})),
                        ("item/fileChange/patchUpdated", json!({"changes":{}})),
                        ("item/commandExecution/terminalInteraction", json!({"processId":"42","stdin":null})),
                    ] {
                        let mut params = json!({"threadId":"codex-session","turnId":"turn1","itemId":"malformed-notification"});
                        params.as_object_mut().unwrap().extend(fields.as_object().unwrap().clone());
                        emit(json!({"method":method,"params":params}));
                    }
                    for (thread, turn) in [("other", "turn1"), ("codex-session", "old-turn"), ("codex-session", "turn1")] {
                        emit(
                            json!({"method":"item/commandExecution/terminalInteraction","params":{"threadId":thread,"turnId":turn,"itemId":"terminal-progress","processId":"process-42","stdin":""}}),
                        );
                    }
                    emit(
                        json!({"method":"item/completed","params":{"threadId":"codex-session","turnId":"turn1","item":{"type":"commandExecution","id":"terminal-progress","command":"fixture-terminal","status":"completed","aggregatedOutput":"actual stdout"}}}),
                    );
                    for (thread, turn, changes) in [
                        ("other", "turn1", json!([])),
                        ("codex-session", "old-turn", json!([])),
                        ("codex-session", "turn1", json!([])),
                    ] {
                        emit(
                            json!({"method":"item/fileChange/patchUpdated","params":{"threadId":thread,"turnId":turn,"itemId":"patch-progress","changes":changes}}),
                        );
                    }
                    emit(
                        json!({"method":"item/completed","params":{"threadId":"codex-session","turnId":"turn1","item":{"type":"fileChange","id":"patch-progress","changes":[],"status":"completed"}}}),
                    );
                    emit(
                        json!({"method":"item/mcpToolCall/progress","params":{"threadId":"codex-session","turnId":"turn1","itemId":"mcp-progress","message":"자료 검증 중"}}),
                    );
                    emit(
                        json!({"method":"item/completed","params":{"threadId":"codex-session","turnId":"turn1","item":{"type":"mcpToolCall","id":"mcp-progress","tool":"lookup","status":"completed","result":{"text":"Found the fixture"}}}}),
                    );
                    for (thread, turn) in [("other", "turn1"), ("codex-session", "old-turn")] {
                        emit(
                            json!({"method":"item/completed","params":{"threadId":thread,"turnId":turn,"item":{"type":"contextCompaction","id":"foreign-compaction"}}}),
                        );
                    }
                    for method in ["item/started", "item/completed"] {
                        emit(
                            json!({"method":method,"params":{"threadId":"codex-session","turnId":"turn1","item":{"type":"contextCompaction","id":"compaction-1"}}}),
                        );
                    }
                }
                emit_tool(provider, "in_progress", None);
                emit(json!({"id":"approval-1","method":"item/commandExecution/requestApproval","params":{"command":"fixture-command"}}));
                let reply: Value = serde_json::from_str(&lines.next().unwrap()?)?;
                ensure!(reply["id"] == "approval-1");
                ensure!(
                    reply["result"]["decision"]
                        == if scenario == "interactive-request-cancel" {
                            "cancel"
                        } else if allows(scenario) {
                            "accept"
                        } else {
                            "decline"
                        }
                );
                if scenario.starts_with("interactive-question") {
                    emit(
                        json!({"id":"question","method":"item/tool/requestUserInput","params":{"questions":[{"id":"q1","question":"Choose?","options":[{"label":"A"},{"label":"B"}]}]}}),
                    );
                    let reply: Value = serde_json::from_str(&lines.next().unwrap()?)?;
                    if scenario == "interactive-question-cancel" {
                        ensure!(reply["result"]["answers"] == json!({}));
                    } else {
                        ensure!(reply["result"]["answers"]["q1"]["answers"] == json!(["B"]));
                    }
                }
                if scenario == "interactive-running" {
                    emit_tool(provider, "in_progress", None);
                    std::thread::park();
                    unreachable!();
                }
                emit_tool(
                    provider,
                    if scenario == "interactive-failed" {
                        "failed"
                    } else if allows(scenario) {
                        "completed"
                    } else {
                        "declined"
                    },
                    Some("fixture result 🦀"),
                );
                emit(json!({"method":"item/agentMessage/delta","params":{"threadId":"other","delta":"wrong"}}));
                for delta in ["Hello ", "세계"] {
                    emit(json!({"method":"item/agentMessage/delta","params":{"threadId":"codex-session","turnId":"turn1","delta":delta}}));
                }
                emit(
                    json!({"method":"thread/tokenUsage/updated","params":{"threadId":"codex-session","turnId":"turn1","tokenUsage":{"total":{"inputTokens":40,"outputTokens":2,"cachedInputTokens":30,"totalTokens":42},"last":{"totalTokens":12},"modelContextWindow":1000}}}),
                );
                emit(
                    json!({"method":"turn/completed","params":{"threadId":"codex-session","turn":{"id":"turn1","status":if scenario == "failed" {"failed"} else {"completed"},"error":{"message":"fixture failure"}}}}),
                );
                json!({"turn":{"id":"turn1"}})
            },
            | "session/new" | "session/resume" => {
                if scenario == "oauth-mcp" && params["mcpServers"].as_array().is_some_and(|v| !v.is_empty()) {
                    ensure!(params["mcpServers"][0]["headers"][0] == json!({"name":"Authorization","value":"Bearer fixture-oauth-access"}));
                }
                if scenario.starts_with("http-mcp") && params["mcpServers"].as_array().is_some_and(|servers| !servers.is_empty()) {
                    let remote = params["mcpServers"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|server| server["name"] == "remote")
                        .context("Remote HTTP MCP missing")?;
                    ensure!(remote["type"] == "http" && remote["url"] == "https://mcp.example.invalid/tools");
                    let headers = remote["headers"].as_array().context("HTTP headers array")?;
                    ensure!(
                        headers.iter().any(|header| header["name"] == "Authorization"
                            && header["value"] == format!("Bearer {}", std::env::var("HTTP_MCP_TEST_TOKEN").unwrap()))
                    );
                    ensure!(
                        headers
                            .iter()
                            .any(|header| header["name"] == "X-Tenant" && header["value"] == std::env::var("HTTP_MCP_TEST_TENANT").unwrap())
                    );
                    if scenario == "http-mcp-only" {
                        let servers = params["mcpServers"].as_array().unwrap();
                        ensure!(servers.len() == 2);
                        let host = servers
                            .iter()
                            .find(|server| server["name"] == "gpui-agent-session")
                            .context("Scoped host MCP missing")?;
                        ensure!(host["type"] == "http" && host["url"] == "http://127.0.0.1:12345/mcp");
                        ensure!(host["headers"] == json!([{"name":"Authorization","value":format!("Bearer session_{}", "a".repeat(64))}]));
                    }
                }
                ensure!(
                    !params["cwd"].as_str().unwrap().starts_with(r"\\?\"),
                    "Antigravity cannot convert Windows verbatim cwd paths into file URIs"
                );
                if !token.exists() {
                    emit(json!({"id":id,"error":{"code":-32000,"message":"authentication required"}}));
                    continue;
                }
                if method == "session/resume" {
                    ensure!(params["sessionId"] == "agy-session");
                }
                let mut session = json!({"configOptions":[{"id":"model","type":"select","options":[{"group":"Gemini","options":[{"value":"gemini-fixture","name":"Gemini"}]}]}]});
                if method == "session/new" {
                    session["sessionId"] = json!("agy-session");
                }
                session
            },
            | "session/set_config_option" => {
                ensure!(params["configId"] == "model" && params["value"] == "gemini-fixture");
                json!({})
            },
            | "session/prompt" => {
                if scenario == "guidance" {
                    check_guidance(params["prompt"][0]["text"].as_str().context("ACP text input")?)?;
                }
                if scenario.starts_with("attachments") {
                    check_attachments(&params["prompt"], provider, scenario)?;
                }
                emit_tool(provider, "in_progress", None);
                emit(
                    json!({"jsonrpc":"2.0","id":"permission","method":"session/request_permission","params":{"toolCall":if spec["computerPermissions"] == true {json!({"_meta":{"mcp":{"server":"gpui-agent-session","tool":"computer_capture"}},"rawInput":{"arguments":{}},"toolCallId":"tool-1"})} else {Value::Null},"options":[{"kind":"allow_once","optionId":"yes"},{"kind":"reject_once","optionId":"no"}]}}),
                );
                let reply: Value = serde_json::from_str(&lines.next().unwrap()?)?;
                if scenario == "interactive-request-cancel" {
                    ensure!(reply["result"]["outcome"]["outcome"] == "cancelled");
                } else {
                    ensure!(reply["result"]["outcome"]["optionId"] == if allows(scenario) { "yes" } else { "no" });
                }
                if scenario == "interactive-running" {
                    emit_tool(provider, "in_progress", None);
                    std::thread::park();
                    unreachable!();
                }
                emit_tool(
                    provider,
                    if scenario == "interactive-failed" {
                        "failed"
                    } else if allows(scenario) {
                        "completed"
                    } else {
                        "failed"
                    },
                    Some("fixture result 🦀"),
                );
                let count = if scenario == "long-stream" { 1500 } else { 2 };
                for i in 0 .. count {
                    emit(
                        json!({"method":"session/update","params":{"sessionId":"agy-session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":if i == 0 {"Hello "} else {"세계"}}}}}),
                    );
                }
                json!({"stopReason":if scenario == "failed" {"cancelled"} else {"end_turn"}})
            },
            | _ => anyhow::bail!("unexpected method: {method}"),
        };
        if !(provider == "codex" && method == "turn/start" && scenario == "progress-events") {
            emit(json!({"id":id,"result":result}));
        }
    }
    Ok(())
}

struct Harness {
    _root: tempfile::TempDir,
    config: Config,
}
impl Harness {
    fn new(provider: Provider, scenario: &str) -> Result<Self> {
        let root = tempfile::tempdir()?;
        let name = match provider {
            | Provider::Claude => "claude",
            | Provider::Codex => "codex",
            | Provider::Antigravity => "antigravity",
        };
        fs::write(root.path().join("fixture.json"), json!({"provider":name,"scenario":scenario}).to_string())?;
        let executable = std::env::current_exe()?;
        let config = Config {
            provider,
            executable: executable.clone(),
            home: root.path().join("home"),
            cwd: root.path().into(),
            harness: Some(executable.clone()),
            browser_helper: Some(executable),
            timeout: Duration::from_secs(10),
            mcp_home: None,
            mcp_selection: None,
            plugin_mcp: vec![],
        };
        Ok(Self {
            _root: root,
            config,
        })
    }

    fn agent(&self) -> Result<Agent> { Agent::new(self.config.clone()) }

    fn sign_in(&self) -> Result<()> {
        let token = if self.config.provider == Provider::Antigravity {
            self.config.home.join("antigravity-acp/acp_token.json")
        } else {
            self.config.home.join("fixture-auth")
        };
        fs::create_dir_all(token.parent().unwrap())?;
        fs::write(token, "fixture")?;
        Ok(())
    }

    fn requests(&self) -> Result<Vec<Value>> {
        Ok(fs::read_to_string(self.config.home.join("requests.jsonl"))?
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect())
    }
}

async fn auth_lifecycle(provider: Provider) -> Result<()> {
    let harness = Harness::new(provider, "normal")?;
    let agent = harness.agent()?;
    ensure!(!agent.status().await?.authenticated);
    let mut authorization = 0;
    ensure!(
        agent
            .login(false, &mut |event| {
                if matches!(event, Event::Authorization { .. }) {
                    authorization += 1;
                }
                false
            })
            .await?
            .authenticated
    );
    if provider != Provider::Claude {
        ensure!(authorization == 1);
    }
    ensure!(agent.status().await?.authenticated);
    ensure!(!agent.models().await?.is_empty());
    agent.logout().await?;
    ensure!(!agent.status().await?.authenticated);
    Ok(())
}

async fn conversation(provider: Provider, scenario: &str) -> Result<()> {
    let harness = Harness::new(provider, scenario)?;
    let agent = harness.agent()?;
    harness.sign_in()?;
    let mut deltas = String::new();
    let mut count = 0;
    let completion = agent
        .prompt(
            &Prompt {
                mcp_servers: None,
                skills: vec![],
                guidance: if scenario == "guidance" { Some(guidance_fixture(0)) } else { None },
                claude_permission_mode: None,
                codex_execution_policy: None,
                attachments: Vec::new(),
                text: "안녕 $() `test`\nsecond line".into(),
                model: if provider == Provider::Antigravity {
                    Some("gemini-fixture".into())
                } else {
                    None
                },
                session_id: None,
            },
            &mut |event| {
                if let Event::Text {
                    delta,
                } = &event
                {
                    count += 1;
                    deltas.push_str(delta);
                }
                matches!(event, Event::Permission { .. }) && scenario == "allow"
            },
        )
        .await;
    if scenario == "failed" || scenario == "exit-failed" {
        ensure!(completion.is_err());
        return Ok(());
    }
    let completion = completion?;
    ensure!(completion.text == deltas);
    if scenario == "long-stream" {
        ensure!(count == 1500);
    } else {
        ensure!(deltas == "Hello 세계", "duplicate or missing deltas: {deltas}");
    }
    let resumed = agent
        .prompt(
            &Prompt {
                mcp_servers: None,
                skills: vec![],
                guidance: if scenario == "guidance" { Some(guidance_fixture(1)) } else { None },
                claude_permission_mode: None,
                codex_execution_policy: None,
                attachments: Vec::new(),
                text: if scenario == "guidance" {
                    "Guidance changed"
                } else {
                    "안녕 $() `test`\nsecond line"
                }
                .into(),
                model: None,
                session_id: Some(completion.session_id),
            },
            &mut |event| matches!(event, Event::Permission { .. }) && scenario == "allow",
        )
        .await?;
    if scenario == "guidance" {
        agent
            .prompt(
                &Prompt {
                    text: "Guidance removed".into(),
                    session_id: Some(resumed.session_id),
                    guidance: Some(guidance_fixture(2)),
                    ..Default::default()
                },
                &mut |_| false,
            )
            .await?;
    }
    if provider != Provider::Claude {
        let method = if provider == Provider::Codex { "thread/resume" } else { "session/resume" };
        ensure!(harness.requests()?.iter().any(|request| request["method"] == method));
    }
    Ok(())
}

pub fn test_arguments(allowed: &[&str]) -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() <= 1 && args.iter().all(|arg| allowed.contains(&arg.as_str())),
        "Unsupported test argument: custom runners do not support libtest filters; use a documented scenario flag"
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    if gpui_agent::run_browser_helper() {
        return Ok(());
    }
    if let Ok(spec) = fs::read_to_string("fixture.json") {
        return fixture(serde_json::from_str(&spec)?);
    }
    test_arguments(&[
        "--guidance-only",
        "--http-mcp",
        "--http-mcp-invalid",
        "--oauth-mcp",
        "--loop-only",
        "--selection-only",
    ])?;
    if std::env::args().any(|arg| arg == "--selection-only") {
        return conversation_mcp_selection().await;
    }
    if std::env::args().any(|arg| arg == "--guidance-only") {
        for provider in [Provider::Claude, Provider::Codex, Provider::Antigravity] {
            conversation(provider, "guidance").await?;
        }
        println!("ok: shared guidance reaches all providers on new and resumed turns");
        return Ok(());
    }
    if std::env::args().any(|arg| arg == "--loop-only") {
        codex_recovery().await?;
        return codex_live_input().await;
    }
    if std::env::args().any(|arg| arg == "--oauth-mcp") {
        return oauth_mcp_providers().await;
    }
    if std::env::args().any(|arg| arg == "--http-mcp") {
        return http_mcp_providers().await;
    }
    if std::env::args().any(|arg| arg == "--http-mcp-invalid") {
        return invalid_http_mcp_credentials().await;
    }
    if std::env::args_os().len() == 1 {
        let filtered = std::process::Command::new(std::env::current_exe()?).arg("__missing_filter__").output()?;
        ensure!(!filtered.status.success(), "Custom runners must reject unsupported filters");
    }
    mcp_configuration().await?;
    conversation_mcp_selection().await?;
    http_mcp_configuration()?;
    oauth_mcp_providers().await?;
    session_mcp_configuration().await?;
    claude_permissions().await?;
    codex_permissions().await?;
    codex_execution_policies().await?;
    codex_elicitation().await?;
    codex_progress().await?;
    codex_recovery().await?;
    codex_live_input().await?;
    for provider in [Provider::Claude, Provider::Codex, Provider::Antigravity] {
        auth_lifecycle(provider).await?;
        println!("ok: {provider:?} login/status/models/logout");
        for scenario in ["normal", "failed", "guidance"] {
            conversation(provider, scenario).await?;
            println!("ok: {provider:?} {scenario} streaming/resume");
        }
    }
    for provider in [Provider::Codex, Provider::Antigravity] {
        conversation(provider, "allow").await?;
    }
    conversation(Provider::Claude, "exit-failed").await?;
    conversation(Provider::Antigravity, "long-stream").await?;
    println!("ok: approvals, nonzero exit, 1500 ACP streaming chunks");
    let harness = Harness::new(Provider::Codex, "normal")?;
    let status = harness
        .agent()?
        .login(true, &mut |event| {
            if let Event::Authorization {
                code, ..
            } = event
            {
                assert_eq!(code.as_deref(), Some("ABCD"));
            }
            false
        })
        .await?;
    ensure!(status.authenticated);
    for scenario in ["bad-json", "crash"] {
        let harness = Harness::new(Provider::Codex, scenario)?;
        let error = harness.agent()?.status().await.unwrap_err();
        ensure!(!error.to_string().contains("secret invalid payload"));
    }
    let mut harness = Harness::new(Provider::Codex, "timeout")?;
    harness.config.timeout = Duration::from_millis(100);
    ensure!(harness.agent()?.status().await.unwrap_err().is::<tokio::time::error::Elapsed>());
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(harness.config.home.join(".operation.lock"))?;
    lock.try_lock()?;
    ensure!(harness.agent()?.status().await.unwrap_err().to_string().contains("another operation"));
    drop(lock);
    println!("ok: device code, malformed JSON, crash, timeout, profile lock release");
    let harness = Harness::new(Provider::Antigravity, "expired")?;
    let agent = harness.agent()?;
    harness.sign_in()?;
    ensure!(!agent.status().await?.authenticated);
    let harness = Harness::new(Provider::Claude, "status-broken")?;
    ensure!(harness.agent()?.status().await.is_err());
    println!("ok: expired Google token and broken Claude status");
    println!("all protocol integration checks passed");
    Ok(())
}

fn check_attachments(content: &Value, provider: &str, scenario: &str) -> Result<()> {
    if let Some(text) = content.as_str() {
        ensure!(user_question(text) == "continue");
        return Ok(());
    }
    let blocks = content.as_array().context("Attachment input must contain blocks")?;
    // Resumed calls send only the new turn; no replay of old attachments.
    if blocks.len() == 1 {
        ensure!(blocks[0]["text"].as_str().map(user_question) == Some("continue"));
        return Ok(());
    }
    let expected = "fn main() { println!(\"첨부 🦀\"); }\n";
    if provider == "antigravity" && scenario != "attachments-fallback" {
        ensure!(blocks.iter().any(|block| block["type"] == "resource" && block["resource"]["text"] == expected));
    } else {
        ensure!(
            blocks
                .iter()
                .filter_map(|block| block["text"].as_str())
                .any(|text| text.contains("첨부 🦀") && text.contains("sample.rs"))
        );
    }
    ensure!(
        blocks.iter().any(|block| block["type"] == "image"
            && match provider {
                | "claude" =>
                    block["source"]["type"] == "base64"
                        && block["source"]["media_type"] == "image/png"
                        && block["source"]["data"].as_str().is_some_and(|data| data.starts_with("iVBOR")),
                | "codex" => block["url"].as_str().is_some_and(|url| url.starts_with("data:image/png;base64,iVBOR")),
                | _ => block["mimeType"] == "image/png" && block["data"].as_str().is_some_and(|data| data.starts_with("iVBOR")),
            }),
        "Native image payload is missing or malformed"
    );
    Ok(())
}

fn user_question(text: &str) -> &str {
    if text.starts_with("Gpui-agent guidance for the current turn.") {
        text.split_once("\n\nUser request:\n").map_or(text, |(_, question)| question)
    } else {
        text
    }
}

async fn claude_permissions() -> Result<()> {
    use gpui_agent::{ClaudePermissionMode,
                     EventHandler,
                     InteractionResponse,
                     ResponseFuture};
    struct Answer(InteractionResponse);
    impl EventHandler for Answer {
        fn emit(&mut self, _: Event) -> bool { false }

        fn respond(&mut self, event: Event) -> ResponseFuture<'_> {
            assert!(matches!(event, Event::Permission { .. }));
            let response = self.0.clone();
            Box::pin(async move { response })
        }
    }
    for mode in ClaudePermissionMode::ALL {
        let harness = Harness::new(Provider::Claude, "interactive-permission-mode")?;
        harness.sign_in()?;
        fs::write(
            harness.config.cwd.join("fixture.json"),
            json!({"provider":"claude","scenario":"interactive-permission-mode","mode":mode}).to_string(),
        )?;
        harness
            .agent()?
            .prompt(
                &Prompt {
                    text: "permission mode".into(),
                    claude_permission_mode: Some(mode),
                    codex_execution_policy: None,
                    ..Default::default()
                },
                &mut Answer(InteractionResponse::ClaudeMode {
                    mode,
                }),
            )
            .await?;
    }
    let harness = Harness::new(Provider::Claude, "interactive-permission-suggestion")?;
    harness.sign_in()?;
    harness
        .agent()?
        .prompt(
            &Prompt {
                text: "permission suggestion".into(),
                ..Default::default()
            },
            &mut Answer(InteractionResponse::ClaudeSuggestion {
                index: 1,
            }),
        )
        .await?;
    println!("ok: all six Claude permission modes and exact runtime permission suggestions");
    Ok(())
}

async fn codex_permissions() -> Result<()> {
    use gpui_agent::{EventHandler,
                     InteractionResponse,
                     ResponseFuture};
    struct Decide {
        response: InteractionResponse,
        requests: usize,
    }
    impl EventHandler for Decide {
        fn emit(&mut self, _: Event) -> bool { false }

        fn respond(&mut self, event: Event) -> ResponseFuture<'_> {
            let response = if let Event::Permission {
                method,
                details,
            } = event
                && method == "item/permissions/requestApproval"
            {
                assert_eq!(details["reason"], "Run fixture");
                assert_eq!(details["permissions"]["network"]["enabled"], true);
                self.requests += 1;
                self.response.clone()
            } else {
                InteractionResponse::Approval {
                    allow: false,
                }
            };
            Box::pin(async move { response })
        }
    }
    for (scenario, response) in [
        (
            "permissions-accept",
            InteractionResponse::Approval {
                allow: true,
            },
        ),
        (
            "permissions-decline",
            InteractionResponse::Approval {
                allow: false,
            },
        ),
        ("permissions-cancel", InteractionResponse::Cancelled),
    ] {
        let harness = Harness::new(Provider::Codex, scenario)?;
        harness.sign_in()?;
        let mut handler = Decide {
            response,
            requests: 0,
        };
        harness
            .agent()?
            .prompt(
                &Prompt {
                    text: "request permissions".into(),
                    ..Default::default()
                },
                &mut handler,
            )
            .await?;
        ensure!(handler.requests == 1, "Given a permission request, the user must be asked exactly once");
    }
    println!("ok: Codex turn permissions accept/decline/cancel without session grants");
    Ok(())
}

async fn codex_execution_policies() -> Result<()> {
    for approval in ["untrusted", "on-request", "never"] {
        for sandbox in ["read-only", "workspace-write", "danger-full-access"] {
            let harness = Harness::new(Provider::Codex, "codex-policy")?;
            harness.sign_in()?;
            let policy = json!({"approval_policy":approval,"sandbox":sandbox});
            fs::write(
                harness.config.cwd.join("fixture.json"),
                json!({"provider":"codex","scenario":"codex-policy","policy":policy}).to_string(),
            )?;
            let agent = harness.agent()?;
            let first = agent
                .prompt(
                    &Prompt {
                        text: "안녕 $() `test`\nsecond line".into(),
                        codex_execution_policy: Some(serde_json::from_value(policy.clone())?),
                        ..Default::default()
                    },
                    &mut |_| false,
                )
                .await?;
            agent
                .prompt(
                    &Prompt {
                        text: "안녕 $() `test`\nsecond line".into(),
                        session_id: Some(first.session_id),
                        codex_execution_policy: Some(serde_json::from_value(policy)?),
                        ..Default::default()
                    },
                    &mut |_| false,
                )
                .await?;
        }
    }
    println!("ok: all nine Codex approval/sandbox combinations on start and resume");
    Ok(())
}

async fn codex_elicitation() -> Result<()> {
    use gpui_agent::{EventHandler,
                     InteractionResponse,
                     ResponseFuture};
    struct Decide {
        action: &'static str,
        requests: usize,
        expected_computer: bool,
    }
    impl EventHandler for Decide {
        fn emit(&mut self, _: Event) -> bool { false }

        fn respond(&mut self, event: Event) -> ResponseFuture<'_> {
            let value = serde_json::to_value(&event).unwrap();
            let response = if value["type"] == "permission" && value["method"] == "computer/native_tool" {
                assert!(self.expected_computer);
                self.requests += 1;
                assert!(event.automatic_computer_response().is_some());
                InteractionResponse::Approval {
                    allow: self.action == "accept",
                }
            } else if value["type"] == "elicitation" && value["server_name"] == "gpui-agent-session" {
                assert!(!self.expected_computer);
                self.requests += 1;
                assert!(event.automatic_computer_response().is_none());
                event.elicitation_response("{}").unwrap()
            } else if value["type"] == "elicitation" {
                assert_eq!(value["server_name"], "fixture-server");
                assert_eq!(value["message"], "Configure the fixture");
                if value["schema"].is_null() {
                    assert_eq!(value["url"], "https://example.invalid/verify");
                } else {
                    assert_eq!(value["schema"]["properties"]["count"]["type"], "integer");
                }
                self.requests += 1;
                if self.action == "cancelled" {
                    InteractionResponse::Cancelled
                } else {
                    serde_json::from_value(json!({"type":"elicitation","action":if self.action == "invalid" { "accept" } else { self.action },"content":
                        if self.action == "invalid" { json!({"count":"two"}) } else if self.action == "accept" && !value["schema"].is_null() { json!({"name":"fixture 🦀","count":2,"enabled":false}) } else { Value::Null }
                    })).expect("Structured elicitation response")
                }
            } else {
                InteractionResponse::Approval {
                    allow: false,
                }
            };
            Box::pin(async move { response })
        }
    }
    for (scenario, action) in [
        ("elicitation-accept", "accept"),
        ("elicitation-decline", "decline"),
        ("elicitation-cancel", "cancel"),
        ("elicitation-cancel", "cancelled"),
        ("elicitation-invalid-cancel", "invalid"),
        ("elicitation-url-accept", "accept"),
        ("elicitation-url-decline", "decline"),
        ("elicitation-url-cancel", "cancel"),
        ("elicitation-computer-accept", "accept"),
        ("elicitation-computer-decline", "decline"),
        ("elicitation-computer-ambiguous-accept", "accept"),
        ("elicitation-computer-invalid-id-accept", "accept"),
    ] {
        let harness = Harness::new(Provider::Codex, scenario)?;
        harness.sign_in()?;
        let mut handler = Decide {
            action,
            requests: 0,
            expected_computer: matches!(scenario, "elicitation-computer-accept" | "elicitation-computer-decline"),
        };
        harness
            .agent()?
            .prompt(
                &Prompt {
                    text: "MCP input".into(),
                    ..Default::default()
                },
                &mut handler,
            )
            .await?;
        ensure!(handler.requests == 1, "Given a form, typed input must be requested from the user");
    }
    let harness = Harness::new(Provider::Codex, "elicitation-url-unsafe")?;
    harness.sign_in()?;
    ensure!(
        harness
            .agent()?
            .prompt(
                &Prompt {
                    text: "Unsafe URL".into(),
                    ..Default::default()
                },
                &mut |_| false
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("Unsupported MCP request URL")
    );
    println!("ok: Codex MCP form/URL accept/decline/cancel, invalid content cancelled and unsafe URLs rejected");
    Ok(())
}

async fn codex_progress() -> Result<()> {
    let harness = Harness::new(Provider::Codex, "progress-events")?;
    harness.sign_in()?;
    let mut events = Vec::new();
    harness
        .agent()?
        .prompt(
            &Prompt {
                text: "Show progress".into(),
                ..Default::default()
            },
            &mut |event| {
                events.push(event);
                false
            },
        )
        .await?;
    let calls: Vec<_> = events
        .iter()
        .filter_map(|event| {
            if let Event::Tool {
                call,
            } = event
            {
                Some(call)
            } else {
                None
            }
        })
        .collect();
    let images: Vec<_> = calls.iter().filter(|call| call.id == "image-view-1").collect();
    ensure!(
        !calls.iter().any(|call| call.id == "malformed-notification"),
        "Malformed progress must not create fabricated tool entries"
    );
    let terminal: Vec<_> = calls
        .iter()
        .filter(|call| call.id == "terminal-progress")
        .flat_map(|call| serde_json::to_value(call).unwrap()["activity"].as_array().cloned().unwrap_or_default())
        .collect();
    ensure!(
        terminal
            == vec![
                json!({"type":"terminal_input","process_id":"process-42","stdin":"계속\nnext line\n"}),
                json!({"type":"terminal_input","process_id":"process-42","stdin":""})
            ],
        "Terminal input must remain separate from stdout, preserving process IDs and empty input: {terminal:?}"
    );
    let patches: Vec<_> = calls.iter().filter(|call| call.id == "patch-progress").collect();
    ensure!(
        patches.len() == 4,
        "Patch updates, including an empty replacement, must remain observable: {patches:?}"
    );
    ensure!(patches[1].input == Some(json!([{"path":"src/fixture.rs","kind":{"type":"update","move_path":null},"diff":"-before\n+after\n"}])));
    ensure!(
        patches[2].input == Some(json!([])) && patches[2].status.is_none(),
        "A patch snapshot replaces changes without completing the tool"
    );
    let activities: Vec<_> = calls
        .iter()
        .filter(|call| call.id == "mcp-progress")
        .flat_map(|call| serde_json::to_value(call).unwrap()["activity"].as_array().cloned().unwrap_or_default())
        .collect();
    ensure!(
        activities
            == vec![
                json!({"type":"progress","message":"자료 검색 중"}),
                json!({"type":"progress","message":"자료 검증 중"})
            ],
        "MCP progress must survive before and after turn/start, excluding other threads and turns: {activities:?}"
    );
    ensure!(images.len() == 2, "Given an image view, its start and completion must both be observable");
    ensure!(images[0].status.as_deref() == Some("in_progress") && images[1].status.as_deref() == Some("completed"));
    ensure!(
        images.iter().all(|call| call.title.as_deref() == Some("imageView")
            && call.output == Some(json!({"type":"imageView","id":"image-view-1","path":"C:/fixture/이미지 preview.png"}))),
        "Image path and native item metadata must survive event conversion"
    );
    let compactions: Vec<_> = calls.iter().filter(|call| call.id == "compaction-1").collect();
    ensure!(
        compactions.len() == 2,
        "Given context compaction after turn/start, its start and completion must both be observable"
    );
    ensure!(compactions[0].status.as_deref() == Some("in_progress") && compactions[1].status.as_deref() == Some("completed"));
    ensure!(
        compactions
            .iter()
            .all(|call| call.title.as_deref() == Some("contextCompaction") && call.output == Some(json!({"type":"contextCompaction","id":"compaction-1"})))
    );
    ensure!(
        !calls.iter().any(|call| call.id == "foreign-compaction"),
        "Progress from another thread or turn must be excluded"
    );
    ensure!(
        calls.iter().filter(|call| call.title.as_deref() == Some("update_plan")).count() == 2,
        "Given plan updates, both revisions must be observable"
    );
    let plan = calls
        .iter()
        .find(|call| call.title.as_deref() == Some("update_plan") && call.status.as_deref() == Some("completed"))
        .context("Completed plan")?;
    ensure!(plan.output.as_ref().unwrap()["plan"][0]["step"] == "Verify");
    let collab: Vec<_> = calls.iter().filter(|call| call.id == "collab-1").collect();
    ensure!(collab.len() == 2 && collab[0].status.as_deref() == Some("in_progress") && collab[1].status.as_deref() == Some("completed"));
    ensure!(collab[1].output.as_ref().unwrap()["agentsStates"]["child-1"]["message"] == "Verified");
    ensure!(collab[1].output.as_ref().unwrap()["receiverThreadIds"] == json!(["child-1"]));
    ensure!(
        calls
            .iter()
            .any(|call| call.id == "plan-item" && call.output_delta.as_deref() == Some("Draft plan"))
    );
    ensure!(
        calls
            .iter()
            .any(|call| call.id == "plan-item" && call.output == Some(json!("Authoritative plan")))
    );
    ensure!(
        calls
            .iter()
            .any(|call| call.id == "activity" && call.output.as_ref().unwrap()["agentThreadId"] == "child-1")
    );
    println!(
        "ok: plan, subagent, image-view, compaction, MCP progress, patch snapshots and terminal input before/after turn/start reply; foreign and malformed notifications excluded"
    );
    Ok(())
}

async fn codex_recovery() -> Result<()> {
    let harness = Harness::new(Provider::Codex, "retry-events")?;
    harness.sign_in()?;
    let mut events = vec![];
    let result = harness
        .agent()?
        .prompt(
            &Prompt {
                text: "Continue".into(),
                ..Default::default()
            },
            &mut |event| {
                events.push(serde_json::to_value(event).unwrap());
                false
            },
        )
        .await?;
    let diagnostics: Vec<_> = events.iter().filter(|event| event["type"] == "runtime_status").collect();
    ensure!(diagnostics.len() == 2, "Recovery and fallback must be observable: {diagnostics:?}");
    ensure!(diagnostics[0]["status"] == "retrying" && diagnostics[0]["message"] == "Reconnecting 1/3");
    ensure!(diagnostics[1]["status"] == "warning");
    ensure!(result.text == "Hello 세계");
    println!("ok: native recovery remains nonterminal, scoped and observable without replaying the turn");
    Ok(())
}

async fn codex_live_input() -> Result<()> {
    use std::sync::{Arc,
                    atomic::{AtomicBool,
                             AtomicUsize,
                             Ordering}};
    struct Input {
        sent: bool,
        rejected: Arc<AtomicBool>,
        reads_after_rejection: Arc<AtomicUsize>,
    }
    impl gpui_agent::TurnInput for Input {
        fn next(&mut self) -> gpui_agent::RuntimeFuture<'_, Option<gpui_agent::RuntimeInput>> {
            if self.rejected.load(Ordering::SeqCst) {
                self.reads_after_rejection.fetch_add(1, Ordering::SeqCst);
            }
            Box::pin(async move {
                let next = (!self.sent).then(|| gpui_agent::RuntimeInput {
                    message_id: 1,
                    text: "Updated instruction".into(),
                });
                self.sent = true;
                Ok(next)
            })
        }
    }
    for accepted in [true, false] {
        let rejected = Arc::new(AtomicBool::new(false));
        let reads_after_rejection = Arc::new(AtomicUsize::new(0));
        let mut input = Input {
            sent: false,
            rejected: rejected.clone(),
            reads_after_rejection: reads_after_rejection.clone(),
        };
        let harness = Harness::new(Provider::Codex, if accepted { "live-input" } else { "live-input-rejected" })?;
        harness.sign_in()?;
        let mut events = vec![];
        let result = harness
            .agent()?
            .prompt_with_input(
                &Prompt {
                    text: "Start".into(),
                    ..Default::default()
                },
                &mut |event| {
                    if matches!(
                        event,
                        Event::InputDelivery {
                            delivered: false,
                            ..
                        }
                    ) {
                        rejected.store(true, Ordering::SeqCst);
                    }
                    events.push(serde_json::to_value(event).unwrap());
                    false
                },
                Some(&mut input),
            )
            .await?;
        ensure!(result.text == "Observed steering");
        ensure!(
            events
                .iter()
                .any(|event| event["type"] == "input_delivery" && event["message_id"] == 1 && event["delivered"] == accepted)
        );
        ensure!(
            reads_after_rejection.load(Ordering::SeqCst) == 0,
            "Rejected steering must leave later inputs in the durable mailbox"
        );
    }
    println!("ok: native steering preserves interleaved output; rejection leaves later inputs available for mailbox delivery");
    Ok(())
}

async fn conversation_mcp_selection() -> Result<()> {
    let common = tempfile::tempdir()?;
    let stored = json!([{"name":"fixture","command":std::env::current_exe()?,"args":[]}]).to_string();
    fs::write(common.path().join("mcp.json"), &stored)?;
    for provider in [Provider::Claude, Provider::Codex, Provider::Antigravity] {
        let mut harness = Harness::new(provider, "mcp-selection")?;
        harness.sign_in()?;
        harness.config.mcp_home = Some(common.path().into());
        harness.config.plugin_mcp = serde_json::from_value(json!([{"name":"packaged","command":std::env::current_exe()?,"args":[]}]))?;
        let agent = harness.agent()?.with_mcp_connection(gpui_agent::McpConnection {
            url: "http://127.0.0.1:1/mcp".into(),
            token: format!("session_{}", "a".repeat(64)),
        });
        let mut session = None;
        for selection in [Some(vec!["fixture".into()]), Some(vec![]), None] {
            let completion = agent
                .prompt(
                    &Prompt {
                        text: "안녕 $() `test`\nsecond line".into(),
                        mcp_servers: selection.clone(),
                        session_id: session,
                        ..Default::default()
                    },
                    &mut |_| false,
                )
                .await
                .with_context(|| format!("MCP selection failed for {provider:?}: {selection:?}"))?;
            session = Some(completion.session_id);
            let configured = if provider == Provider::Claude {
                let value: Value = serde_json::from_slice(&fs::read(harness.config.home.join("claude-mcp.json"))?)?;
                value["mcpServers"].as_object().unwrap().keys().cloned().collect::<Vec<_>>()
            } else {
                let requests = harness.requests()?;
                let request = requests
                    .iter()
                    .rev()
                    .find(|request| {
                        matches!(
                            request["method"].as_str(),
                            Some("thread/start" | "thread/resume" | "session/new" | "session/resume")
                        )
                    })
                    .unwrap();
                if provider == Provider::Codex {
                    request["params"]["config"]
                        .as_object()
                        .unwrap()
                        .keys()
                        .filter_map(|key| key.strip_prefix("mcp_servers.").and_then(|key| key.strip_suffix(".command")).map(str::to_owned))
                        .collect()
                } else {
                    request["params"]["mcpServers"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|server| server["name"].as_str().unwrap().to_owned())
                        .collect()
                }
            };
            ensure!(configured.iter().any(|name| name == "gpui-agent-session"), "host tools lost for {provider:?}");
            ensure!(
                configured.iter().any(|name| name == "fixture") == selection.as_ref().is_none_or(|names| names.iter().any(|name| name == "fixture")),
                "standalone selection mismatch for {provider:?}"
            );
            ensure!(
                configured.iter().any(|name| name == "packaged") == selection.is_none(),
                "package selection mismatch for {provider:?}"
            );
            ensure!(
                fs::read_to_string(common.path().join("mcp.json"))? == stored && !harness.config.home.join("mcp.json").exists(),
                "selection rewrote installed configuration"
            );
        }
    }
    println!("ok: one common MCP registration, conversation selection, empty deselection and unchanged configuration on all native providers");
    Ok(())
}

async fn mcp_configuration() -> Result<()> {
    for provider in [Provider::Claude, Provider::Codex, Provider::Antigravity] {
        let harness = Harness::new(provider, "normal")?;
        let agent = harness.agent()?;
        harness.sign_in()?;
        fs::write(
            harness.config.home.join("mcp.json"),
            json!([{"name":"fixture","command":std::env::current_exe()?,"args":["literal $() `arg`", "한글"]}]).to_string(),
        )?;
        let prompt = Prompt {
            text: "안녕 $() `test`\nsecond line".into(),
            ..Default::default()
        };
        let first = agent.prompt(&prompt, &mut |_| false).await?;
        agent
            .prompt(
                &Prompt {
                    session_id: Some(first.session_id),
                    ..prompt
                },
                &mut |_| false,
            )
            .await?;
        if provider == Provider::Antigravity {
            ensure!(
                harness
                    .requests()?
                    .iter()
                    .filter(|r| r["method"] == "session/new" || r["method"] == "session/resume")
                    .filter(|r| r["params"]["mcpServers"].as_array().is_some_and(|a| !a.is_empty()))
                    .count()
                    == 2
            );
        }
    }
    let harness = Harness::new(Provider::Antigravity, "mcp-unsupported")?;
    let agent = harness.agent()?;
    harness.sign_in()?;
    fs::write(
        harness.config.home.join("mcp.json"),
        json!([{"name":"fixture","command":std::env::current_exe()?,"args":[]}]).to_string(),
    )?;
    ensure!(agent.status().await?.authenticated);
    let error = agent
        .prompt(
            &Prompt {
                text: "test".into(),
                ..Default::default()
            },
            &mut |_| false,
        )
        .await
        .unwrap_err();
    ensure!(error.to_string().contains("does not advertise MCP stdio"));
    println!("ok: MCP configuration on all provider start/resume paths and unsupported Antigravity rejection without breaking login/status");
    Ok(())
}

fn http_mcp_configuration() -> Result<()> {
    // Give the isolated test host its own environment instead of mutating the
    // test runner's globals.
    let result = std::process::Command::new(std::env::current_exe()?)
        .arg("--http-mcp")
        .env("HTTP_MCP_TEST_TOKEN", "fixture-token-not-a-real-credential")
        .env("HTTP_MCP_TEST_TENANT", "fixture-tenant")
        .output()?;
    ensure!(
        result.status.success(),
        "HTTP MCP provider boundary failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    println!("{}", String::from_utf8_lossy(&result.stdout).trim());
    for value in [None, Some(""), Some("sensitive-value\r\nInjected: bad"), Some("${DO_NOT_EXPAND_SECRET}")] {
        let mut command = std::process::Command::new(std::env::current_exe()?);
        command.arg("--http-mcp-invalid").env_remove("HTTP_MCP_TEST_TOKEN");
        if let Some(value) = value {
            command.env("HTTP_MCP_TEST_TOKEN", value);
        }
        let result = command.output()?;
        ensure!(
            result.status.success(),
            "HTTP MCP invalid-credential boundary failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    println!("ok: missing/empty/injected MCP header credentials fail before connecting and never appear in errors");
    Ok(())
}

async fn invalid_http_mcp_credentials() -> Result<()> {
    for provider in [Provider::Codex, Provider::Claude, Provider::Antigravity] {
        let harness = Harness::new(provider, "http-mcp")?;
        harness.sign_in()?;
        fs::write(
            harness.config.home.join("mcp.json"),
            json!([{"name":"remote","url":"https://mcp.example.invalid/tools","bearer_token_env_var":"HTTP_MCP_TEST_TOKEN"}]).to_string(),
        )?;
        let agent = harness.agent()?;
        ensure!(agent.status().await?.authenticated);
        let error = agent
            .prompt(
                &Prompt {
                    text: "안녕 $() `test`\nsecond line".into(),
                    ..Default::default()
                },
                &mut |_| false,
            )
            .await
            .unwrap_err()
            .to_string();
        ensure!(error.contains("MCP HTTP environment variable HTTP_MCP_TEST_TOKEN"));
        if let Ok(value) = std::env::var("HTTP_MCP_TEST_TOKEN")
            && !value.is_empty()
        {
            ensure!(!error.contains(&value));
        }
    }
    Ok(())
}

async fn http_mcp_providers() -> Result<()> {
    for provider in [Provider::Codex, Provider::Claude, Provider::Antigravity] {
        let scenarios: &[&str] = if provider == Provider::Antigravity {
            &["http-mcp", "http-mcp-v2", "http-mcp-only"]
        } else {
            &["http-mcp"]
        };
        for scenario in scenarios {
            let harness = Harness::new(provider, scenario)?;
            harness.sign_in()?;
            let mut config = json!([
                {"name":"fixture","command":std::env::current_exe()?,"args":["--stdio"]},
                {"name":"remote","url":"https://mcp.example.invalid/tools","bearer_token_env_var":"HTTP_MCP_TEST_TOKEN","env_http_headers":{"X-Tenant":"HTTP_MCP_TEST_TENANT"}}
            ]);
            if *scenario == "http-mcp-only" {
                config.as_array_mut().unwrap().remove(0);
            }
            fs::write(harness.config.home.join("mcp.json"), config.to_string())?;
            // Given an HTTP-only ACP, scoped host tools must survive both new
            // and resumed sessions without requiring stdio support.
            let agent = harness.agent()?.with_mcp_connection(gpui_agent::McpConnection {
                url: "http://127.0.0.1:12345/mcp".into(),
                token: format!("session_{}", "a".repeat(64)),
            });
            let first = agent
                .prompt(
                    &Prompt {
                        text: "안녕 $() `test`\nsecond line".into(),
                        ..Default::default()
                    },
                    &mut |_| false,
                )
                .await?;
            agent
                .prompt(
                    &Prompt {
                        text: "안녕 $() `test`\nsecond line".into(),
                        session_id: Some(first.session_id),
                        ..Default::default()
                    },
                    &mut |_| false,
                )
                .await?;
            let recorded = fs::read_to_string(
                harness
                    .config
                    .home
                    .join(if provider == Provider::Claude { "claude-mcp.json" } else { "requests.jsonl" }),
            )?;
            ensure!(recorded.contains("remote"));
            ensure!(recorded.contains("gpui-agent-session"));
            ensure!(!recorded.contains(&format!("session_{}", "a".repeat(64))));
            ensure!(!recorded.contains(&std::env::var("HTTP_MCP_TEST_TOKEN")?));
            if provider == Provider::Claude {
                let starts = fs::read_to_string(harness.config.home.join("claude-http-starts.jsonl"))?;
                let starts: Vec<Value> = starts.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
                ensure!(starts.len() == 2 && starts[0]["resume"] == false && starts[1]["resume"] == true);
                ensure!(
                    starts
                        .iter()
                        .all(|start| !std::path::Path::new(start["config_path"].as_str().unwrap()).exists()),
                    "Temporary credential files must be removed after each turn"
                );
            }
        }
    }
    let harness = Harness::new(Provider::Antigravity, "http-mcp-unsupported")?;
    harness.sign_in()?;
    fs::write(
        harness.config.home.join("mcp.json"),
        json!([{"name":"remote","url":"https://mcp.example.invalid/tools"}]).to_string(),
    )?;
    ensure!(harness.agent()?.status().await?.authenticated);
    let error = harness
        .agent()?
        .prompt(
            &Prompt {
                text: "HTTP capability".into(),
                ..Default::default()
            },
            &mut |_| false,
        )
        .await
        .unwrap_err();
    ensure!(error.to_string().contains("does not advertise MCP HTTP"));
    ensure!(
        !harness
            .requests()?
            .iter()
            .any(|request| request["method"] == "session/new" && request["params"]["mcpServers"].as_array().is_some_and(|servers| !servers.is_empty()))
    );
    println!("ok: HTTP MCP mixed with stdio/session tools on native start and resume without credential diagnostics");
    Ok(())
}

async fn oauth_mcp_providers() -> Result<()> {
    for (provider, plugin) in PROVIDERS_FOR_OAUTH.into_iter().flat_map(|provider| [(provider, false), (provider, true)]) {
        let mut harness = Harness::new(provider, "oauth-mcp")?;
        let harness = tokio::task::spawn_blocking(move || -> Result<Harness> {
            let fixture = oauth_fixture::OAuthFixture::new("valid")?;
            fs::create_dir_all(&harness.config.home)?;
            if plugin {
                harness.config.plugin_mcp = vec![fixture.server()];
            } else {
                fs::write(harness.config.home.join("mcp.json"), serde_json::to_vec(&vec![fixture.server()])?)?;
            }
            fixture.login(&harness.config.home, &gpui_agent::McpOAuth::default())?;
            Ok(harness)
        })
        .await??;
        harness.sign_in()?;
        let agent = harness.agent()?;
        let first = agent
            .prompt(
                &Prompt {
                    text: "안녕 $() `test`\nsecond line".into(),
                    ..Default::default()
                },
                &mut |_| false,
            )
            .await?;
        agent
            .prompt(
                &Prompt {
                    text: "안녕 $() `test`\nsecond line".into(),
                    session_id: Some(first.session_id),
                    ..Default::default()
                },
                &mut |_| false,
            )
            .await?;
        let diagnostic = fs::read_to_string(
            harness
                .config
                .home
                .join(if provider == Provider::Claude { "claude-mcp.json" } else { "requests.jsonl" }),
        )?;
        ensure!(!diagnostic.contains("fixture-oauth-access") && !diagnostic.contains("fixture-oauth-refresh"));
    }
    println!("ok: saved OAuth credentials reach all native start/resume paths without leaking into arguments or recorded configuration");
    Ok(())
}
const PROVIDERS_FOR_OAUTH: [Provider; 3] = [Provider::Codex, Provider::Claude, Provider::Antigravity];

async fn session_mcp_configuration() -> Result<()> {
    for provider in [Provider::Claude, Provider::Codex, Provider::Antigravity] {
        let harness = Harness::new(provider, "interactive-allow")?;
        harness.sign_in()?;
        let connection = gpui_agent::McpConnection {
            url: "http://127.0.0.1:12345/mcp".into(),
            token: format!("session_{}", "a".repeat(64)),
        };
        let agent = harness.agent()?.with_mcp_connection(connection.clone());
        let prompt = Prompt {
            text: "automatic MCP".into(),
            ..Prompt::default()
        };
        let mut handler = |_: Event| true;
        let completed = agent.prompt(&prompt, &mut handler).await?;
        let resumed = Prompt {
            session_id: Some(completed.session_id),
            ..prompt
        };
        agent.prompt(&resumed, &mut handler).await?;
        let recorded = if provider == Provider::Claude {
            fs::read_to_string(harness.config.home.join("claude-mcp.json"))?
        } else {
            fs::read_to_string(harness.config.home.join("requests.jsonl"))?
        };
        ensure!(recorded.contains("gpui-agent-session"), "automatic MCP missing for {provider:?}");
        ensure!(!recorded.contains(&connection.token), "credential leaked into fixture diagnostics");
        ensure!(
            !harness.config.home.join("mcp.json").exists(),
            "automatic credentials must not change profile settings"
        );
    }
    // Given an injected host whose tools failed discovery, when a turn starts,
    // then fail before inference instead of silently running without the tools.
    let harness = Harness::new(Provider::Codex, "missing-host-mcp")?;
    harness.sign_in()?;
    let agent = harness.agent()?.with_mcp_connection(gpui_agent::McpConnection {
        url: "http://127.0.0.1:12345/mcp".into(),
        token: format!("session_{}", "a".repeat(64)),
    });
    let result = agent
        .prompt(
            &Prompt {
                text: "missing host tools".into(),
                ..Default::default()
            },
            &mut |_| false,
        )
        .await;
    ensure!(result.is_err(), "A missing host MCP catalog must prevent inference");
    ensure!(result.unwrap_err().to_string().contains("Host MCP tools are unavailable"));
    let recorded = fs::read_to_string(harness.config.home.join("requests.jsonl"))?;
    ensure!(
        !recorded
            .lines()
            .any(|line| serde_json::from_str::<Value>(line).unwrap()["method"] == "turn/start")
    );
    Ok(())
}

fn redact_session_credentials(value: &mut Value) {
    match value {
        | Value::Object(object) => {
            if let Some(headers) = object.get_mut("headers") {
                match headers {
                    | Value::Object(headers) => headers.values_mut().for_each(|value| *value = json!("[redacted]")),
                    | Value::Array(headers) => headers.iter_mut().for_each(|header| header["value"] = json!("[redacted]")),
                    | _ => {},
                }
            }
            if object.get("name").and_then(Value::as_str) == Some("GPUI_AGENT_MCP_TOKEN") {
                object.insert("value".into(), json!("[redacted]"));
            }
            for value in object.values_mut() {
                redact_session_credentials(value);
            }
        },
        | Value::Array(values) =>
            for value in values {
                redact_session_credentials(value);
            },
        | _ => {},
    }
}

fn probe_session_mcp(server: &Value, provider: &str) -> Result<()> {
    use std::process::{Command,
                       Stdio};
    let mut command = Command::new(server["command"].as_str().context("bridge command")?);
    for argument in server["args"].as_array().context("bridge arguments")? {
        command.arg(argument.as_str().context("bridge argument")?);
    }
    if let Some(environment) = server["env"].as_array() {
        for entry in environment {
            command.env(
                entry["name"].as_str().context("environment name")?,
                entry["value"].as_str().context("environment value")?,
            );
        }
    }
    let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let mut input = child.stdin.take().context("bridge input")?;
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_jobs","arguments":{}}})
    )?;
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"computer_capture","arguments":{}}})
    )?;
    drop(input);
    let output = child.wait_with_output()?;
    ensure!(output.status.success(), "session MCP bridge failed");
    let mut replies = std::io::BufRead::lines(std::io::BufReader::new(output.stdout.as_slice()));
    let reply: Value = serde_json::from_str(&replies.next().context("Missing scoped job response")??)?;
    ensure!(reply["result"]["isError"] == false, "provider session could not call its MCP endpoint");
    ensure!(
        reply["result"]["structuredContent"]["jobs"]
            .as_array()
            .context("scoped jobs")?
            .iter()
            .all(|job| job["scope"]["provider"] == provider)
    );
    let computer: Value = serde_json::from_str(&replies.next().context("Missing computer authorization response")??)?;
    ensure!(
        computer["result"]["isError"] == true && computer["result"]["content"][0]["text"] == "Windows computer use is unavailable on this host",
        "Native execution must be bound to its conversation and durable job: {computer}"
    );
    Ok(())
}

fn guidance_fixture(phase: usize) -> gpui_agent::PreparedGuidance {
    let mut snapshot = gpui_agent::GuidanceSnapshot {
        revision: format!("fixture-{phase}"),
        ..Default::default()
    };
    if phase < 2 {
        snapshot.add_instruction(
            "repo:AGENTS.md".into(),
            if phase == 0 { "SHARED_GUIDANCE_MARKER" } else { "SHARED_GUIDANCE_CHANGED" }.into(),
        );
    }
    gpui_agent::PreparedGuidance {
        extensions: None,
        snapshot,
        selected: vec![],
        implicit_available: false,
    }
}

fn check_guidance(text: &str) -> Result<()> {
    ensure!(text.contains("replaces all earlier gpui-agent guidance"));
    let (_, request) = text.split_once("\n\nUser request:\n").context("Original user request missing")?;
    let payload: Value = serde_json::from_str(text.lines().nth(1).context("Guidance JSON missing")?)?;
    match request {
        | "안녕 $() `test`\nsecond line" => {
            ensure!(payload["revision"] == "fixture-0");
            ensure!(payload["agents"][0]["content"] == "SHARED_GUIDANCE_MARKER");
        },
        | "Guidance changed" => {
            ensure!(payload["revision"] == "fixture-1");
            ensure!(payload["agents"][0]["content"] == "SHARED_GUIDANCE_CHANGED");
            ensure!(!text.contains("SHARED_GUIDANCE_MARKER"));
        },
        | "Guidance removed" => {
            ensure!(payload["revision"] == "fixture-2");
            ensure!(payload["agents"] == json!([]));
            ensure!(!text.contains("SHARED_GUIDANCE_MARKER") && !text.contains("SHARED_GUIDANCE_CHANGED"));
        },
        | _ => anyhow::bail!("Original user request changed"),
    }
    Ok(())
}
