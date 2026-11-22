use super::*;

pub(super) fn tools() -> Vec<ToolDefinition> {
    let id = json!({"type":"string","minLength":1,"maxLength":256});
    vec![
        ToolDefinition {
            name: "device_list",
            description: "List this scope's operator-granted Android devices through the local ADB server. Operators see all attached devices. available=false means ADB is not installed/configured. Does not grant access or start an emulator. iOS, remote device hosts and live streaming are unavailable.",
            schema: json!({"type":"object","properties":{},"additionalProperties":false}),
            read_only: true,
        },
        ToolDefinition {
            name: "device_open",
            description: "Claim an already connected Android device granted to your provider/account/workspace. The operator must first POST /v1/control/devices/grant; grants last 10 minutes. Cannot grant yourself access, boot devices or bypass Android authorization. Call device_screenshot before input.",
            schema: json!({"type":"object","properties":{"device_id":id},"required":["device_id"],"additionalProperties":false}),
            read_only: false,
        },
        ToolDefinition {
            name: "device_screenshot",
            description: "Return the PNG screen of your open, granted Android device as a native image block, with pixel dimensions and a new revision. Up to 2 MiB PNG and 32 million pixels. Capture may fail for secure/protected content. The device adapter creates no capture files; provider tool history may retain returned images. Use the latest revision exactly once with device_act.",
            schema: json!({"type":"object","properties":{"device_id":id},"required":["device_id"],"additionalProperties":false}),
            read_only: true,
        },
        ToolDefinition {
            name: "device_act",
            description: "Perform one tap/swipe/text/key action on your open Android device using the latest screenshot revision. Coordinates are pixels within that capture. Revision is consumed before delivery; after uncertain failure capture again rather than replay. Text accepts printable ASCII except percent (1–1024 bytes), no Unicode. Never type secrets through chat. No arbitrary shell, app install, device shutdown, pairing or network configuration.",
            schema: json!({"type":"object","properties":{"device_id":id,"revision":{"type":"string","minLength":1,"maxLength":80},"action":{"oneOf":[{"type":"object","properties":{"type":{"const":"tap"},"x":{"type":"integer","minimum":0,"maximum":32768},"y":{"type":"integer","minimum":0,"maximum":32768}},"required":["type","x","y"],"additionalProperties":false},{"type":"object","properties":{"type":{"const":"swipe"},"x":{"type":"integer","minimum":0,"maximum":32768},"y":{"type":"integer","minimum":0,"maximum":32768},"to_x":{"type":"integer","minimum":0,"maximum":32768},"to_y":{"type":"integer","minimum":0,"maximum":32768},"duration_ms":{"type":"integer","minimum":1,"maximum":3000}},"required":["type","x","y","to_x","to_y","duration_ms"],"additionalProperties":false},{"type":"object","properties":{"type":{"const":"text"},"text":{"type":"string","minLength":1,"maxLength":1024}},"required":["type","text"],"additionalProperties":false},{"type":"object","properties":{"type":{"const":"key"},"key":{"enum":["Home","Back","Enter","Tab","Backspace","ArrowUp","ArrowDown","ArrowLeft","ArrowRight"]}},"required":["type","key"],"additionalProperties":false}]}},"required":["device_id","revision","action"],"additionalProperties":false}),
            read_only: false,
        },
        ToolDefinition {
            name: "device_close",
            description: "Release only your own Android device session and grant. Repeating close is safe. Does not stop the emulator, disconnect ADB or power off hardware. A new open requires a fresh operator grant.",
            schema: json!({"type":"object","properties":{"device_id":id},"required":["device_id"],"additionalProperties":false}),
            read_only: false,
        },
    ]
}

pub(super) async fn execute(api: &Api, name: &str, mut args: Value, access: &Access) -> Result<Executed, ApplicationError> {
    if args.get("type").is_some() {
        return Err("Unknown device argument: type".into());
    }
    args["type"] = json!(match name {
        | "device_screenshot" => "screenshot",
        | "device_list" => "list",
        | "device_open" => "open",
        | "device_act" => "act",
        | "device_close" => "close",
        | _ => return Err("Unknown device tool".into()),
    });
    let backend = api
        .controls
        .as_deref()
        .ok_or(ApplicationError(ErrorKind::Unavailable, "Device control is unavailable".into()))?;
    match gpui_agent_application::control_device(backend, access.scope.as_ref(), decode(args)?).await? {
        | DeviceReply::Screenshot {
            device_id,
            revision,
            width,
            height,
            image,
        } => Ok(Executed {
            value: json!({"device_id":device_id,"revision":revision,"width":width,"height":height}),
            job: None,
            image: Some(image),
        }),
        | reply => encode(reply, None),
    }
}
