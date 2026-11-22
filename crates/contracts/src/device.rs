use crate::{Attachment,
            McpScope};
use serde::{Deserialize,
            Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub state: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceList {
    pub available: bool,
    pub devices: Vec<DeviceInfo>,
    pub open: Option<String>,
    pub enabled_until: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeviceAction {
    Tap { x: u32, y: u32 },
    Swipe { x: u32, y: u32, to_x: u32, to_y: u32, duration_ms: u32 },
    Text { text: String },
    Key { key: String },
}

impl DeviceAction {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            | Self::Tap {
                x,
                y,
            } if *x > 32768 || *y > 32768 => Err("Invalid device coordinates"),
            | Self::Swipe {
                x,
                y,
                to_x,
                to_y,
                duration_ms,
            } if [x, y, to_x, to_y].into_iter().any(|v| *v > 32768) || !(1 ..= 3000).contains(duration_ms) =>
                Err("Invalid swipe coordinates or duration (1–3000 ms)"),
            | Self::Text {
                text,
            } if text.is_empty() || text.len() > 1024 || !text.bytes().all(|b| (32 ..= 126).contains(&b) && b != b'%') =>
                Err("Device text supports 1–1024 printable ASCII bytes except percent; Unicode input is unavailable"),
            | Self::Key {
                key,
            } if !matches!(
                key.as_str(),
                "Home" | "Back" | "Enter" | "Tab" | "Backspace" | "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight"
            ) =>
                Err("Unsupported device key"),
            | _ => Ok(()),
        }
    }

    pub fn validate_frame(&self, width: u32, height: u32) -> Result<(), &'static str> {
        let point = |x: u32, y: u32| x < width && y < height;
        match self {
            | Self::Tap {
                x,
                y,
            } if !point(*x, *y) => Err("Coordinates exceed the captured device screen"),
            | Self::Swipe {
                x,
                y,
                to_x,
                to_y,
                ..
            } if !point(*x, *y) || !point(*to_x, *to_y) => Err("Coordinates exceed the captured device screen"),
            | _ => self.validate(),
        }
    }
}

pub fn validate_device_id(id: &str) -> Result<(), &'static str> {
    if id.is_empty() || id.len() > 256 || !id.as_bytes()[0].is_ascii_alphanumeric() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b)) {
        return Err("Invalid device ID; use an ID from device_list");
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeviceCommand {
    List {},
    Open {
        device_id: String,
    },
    Screenshot {
        device_id: String,
    },
    Act {
        device_id: String,
        revision: String,
        action: DeviceAction,
    },
    Close {
        device_id: String,
    },
}
impl DeviceCommand {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            | Self::List {} => Ok(()),
            | Self::Act {
                device_id,
                revision,
                action,
            } => {
                validate_device_id(device_id)?;
                if revision.is_empty() || revision.len() > 80 {
                    return Err("Invalid device screenshot revision");
                }
                action.validate()
            },
            | Self::Open {
                device_id,
            }
            | Self::Screenshot {
                device_id,
            }
            | Self::Close {
                device_id,
            } => validate_device_id(device_id),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeviceReply {
    List(DeviceList),
    Opened {
        device: DeviceInfo,
        enabled_until: u64,
    },
    Screenshot {
        device_id: String,
        revision: String,
        width: u32,
        height: u32,
        image: Attachment,
    },
    Acted {
        device_id: String,
    },
    Closed {},
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceGrantRequest {
    pub device_id: String,
    pub scope: Option<McpScope>,
    pub enabled: bool,
}
impl DeviceGrantRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_device_id(&self.device_id)?;
        if let Some(scope) = &self.scope {
            crate::validate_account_id(&scope.account_id)?;
            if scope.workspace_path.is_empty() || scope.workspace_path.len() > 32768 || scope.workspace_path.chars().any(char::is_control) {
                return Err("Invalid device grant workspace");
            }
        }
        Ok(())
    }
}

pub fn device_png_dimensions(bytes: &[u8]) -> Result<(u32, u32), &'static str> {
    if bytes.len() < 33 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes[8 .. 12] != [0, 0, 0, 13] || &bytes[12 .. 16] != b"IHDR" {
        return Err("Device screenshot is not PNG");
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if width == 0 || height == 0 || width > 16384 || height > 16384 || u64::from(width) * u64::from(height) > 32_000_000 {
        return Err("Invalid device screenshot dimensions");
    }
    Ok((width, height))
}
