use super::*;

pub(super) fn tools() -> Vec<ToolDefinition> {
    let empty = json!({"type":"object","properties":{},"additionalProperties":false});
    let revision = json!({"type":"string","minLength":1,"maxLength":80});
    let target = json!({"type":"integer","minimum":0,"maximum":99});
    vec![
        ToolDefinition {
            name: "browser_open",
            description: "Open HTTP(S) in an owned temporary Chromium profile isolated by provider/account/workspace and from manual browser controls. Same-scope sessions share one page. Returns bounded visible text and numbered interactive targets. No personal browser profiles, arbitrary JavaScript or desktop access. Use browser_act with the latest revision and target.",
            schema: json!({"type":"object","properties":{"url":{"type":"string","maxLength":8192},"mobile":{"type":"boolean","default":false}},"required":["url"],"additionalProperties":false}),
            read_only: false,
        },
        ToolDefinition {
            name: "browser_render_html",
            description: "Render a self-contained HTML result (1–524288 UTF-8 bytes) in this scope's fresh temporary browser profile. Inline CSS/JavaScript and data/blob images are supported. A sandbox blocks parent access, storage, forms, popups and downloads; CSP, request blocking and a non-forwarding proxy prevent host/external network access. External scripts/styles/fonts and relative files are unavailable: inline assets. Replaces this scope's previous browser, clearing its profile, diagnostics and target references. Returns the rendered DOM snapshot. Use browser_act/wait/snapshot/capture and browser_diagnostics to inspect the result; no arbitrary inspection JavaScript. mobile defaults to false.",
            schema: json!({"type":"object","properties":{"html":{"type":"string","minLength":1,"maxLength":BROWSER_HTML_LIMIT},"mobile":{"type":"boolean","default":false}},"required":["html"],"additionalProperties":false}),
            read_only: false,
        },
        ToolDefinition {
            name: "browser_diagnostics",
            description: "Read this scope's recent browser console messages, unhandled JavaScript errors and browser security/network logs without clearing them or invalidating targets. At most 100 entries, 2000 Unicode characters of text and 512 of URL each; truncated reports lost entries or clipped fields. Lines are one-based. Object arguments are descriptions, not recursive inspection. Includes all observed documents in this owned page; no personal browser or other scope logs. Collection happens while commands run; this is a bounded sample, not a durable or complete audit log. Render/close clears diagnostics.",
            schema: empty.clone(),
            read_only: true,
        },
        ToolDefinition {
            name: "browser_snapshot",
            description: "Read this scope's browser: URL/title, at most 16000 Unicode characters of visible text and 100 visible interactive targets. Input values and password contents are omitted. Each snapshot issues a new revision; previous target references become stale. Reads the main document for URLs or the owned sandbox frame for HTML results; no arbitrary iframe or popup switching.",
            schema: empty.clone(),
            read_only: true,
        },
        ToolDefinition {
            name: "browser_act",
            description: "Perform one browser action, then return a fresh snapshot. click/fill require revision and target from the latest snapshot; stale, hidden, disabled or detached targets are rejected. Fill replaces a writable input/textarea value (up to 4096 UTF-8 bytes). key and scroll operate on the current page. Actions are not idempotent; inspect state before retrying uncertain delivery.",
            schema: json!({"type":"object","properties":{"action":{"oneOf":[{"type":"object","properties":{"type":{"const":"click"},"revision":revision,"target":target},"required":["type","revision","target"],"additionalProperties":false},{"type":"object","properties":{"type":{"const":"fill"},"revision":revision,"target":target,"text":{"type":"string","maxLength":4096}},"required":["type","revision","target","text"],"additionalProperties":false},{"type":"object","properties":{"type":{"const":"key"},"key":{"enum":["Enter","Tab","Escape","Backspace","ArrowUp","ArrowDown","ArrowLeft","ArrowRight","Home","End","Delete"]}},"required":["type","key"],"additionalProperties":false},{"type":"object","properties":{"type":{"const":"scroll"},"delta":{"type":"integer","minimum":-2400,"maximum":2400}},"required":["type","delta"],"additionalProperties":false}]}},"required":["action"],"additionalProperties":false}),
            read_only: false,
        },
        ToolDefinition {
            name: "browser_wait",
            description: "Wait up to timeout_ms (0–10000, default 1000) for visible body text to contain text, then return matched and a fresh snapshot. matched=false is a normal timeout and preserves the browser. No fixed sleep is required. Text is a literal substring, not JavaScript or regex.",
            schema: json!({"type":"object","properties":{"text":{"type":"string","minLength":1,"maxLength":4096},"timeout_ms":{"type":"integer","minimum":0,"maximum":10000,"default":1000}},"required":["text"],"additionalProperties":false}),
            read_only: true,
        },
        ToolDefinition {
            name: "browser_capture",
            description: "Capture this scope's browser viewport as a native MCP image content block with URL/size metadata. Does not invalidate target references. Captures no manual browser or Windows desktop.",
            schema: empty.clone(),
            read_only: true,
        },
        ToolDefinition {
            name: "browser_close",
            description: "Close only this scope's automation browser, cancel pending commands, and discard its temporary profile. Repeated close is safe. Browser state is not durable across host restarts.",
            schema: empty,
            read_only: false,
        },
    ]
}

pub(super) async fn execute(api: &Api, name: &str, mut args: Value, access: &Access) -> Result<Executed, ApplicationError> {
    let kind = name.strip_prefix("browser_").ok_or("Unknown browser tool")?;
    if args.get("type").is_some() {
        return Err("Unknown browser argument: type".into());
    }
    args["type"] = json!(kind);
    let command = decode(args)?;
    let backend = api
        .controls
        .as_deref()
        .ok_or(ApplicationError(ErrorKind::Unavailable, "Browser automation is unavailable".into()))?;
    match gpui_agent_application::automate_browser(backend, access.scope.as_ref(), command).await? {
        | BrowserReply::Snapshot(snapshot) => encode(snapshot, None),
        | BrowserReply::Wait {
            matched,
            snapshot,
        } => encode(json!({"matched":matched,"snapshot":snapshot}), None),
        | BrowserReply::Capture(frame) => Ok(Executed {
            value: json!({"width":frame.width,"height":frame.height,"url":frame.url}),
            job: None,
            image: Some(frame.image),
        }),
        | BrowserReply::Closed => encode(json!({"closed":true}), None),
        | BrowserReply::Diagnostics(value) => encode(value, None),
        | BrowserReply::Rejected(_) => unreachable!("application rejects invalid browser actions"),
    }
}
